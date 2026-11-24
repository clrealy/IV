// IV — vi, but backwards (and simple)
// Modal editor: Normal / Insert / Command, plus Ctrl+S / Ctrl+Q for normal people.

use std::{
    env, fs,
    io::{self, Stdout, Write},
    path::PathBuf,
};

use crossterm::{
    cursor::{MoveTo, SetCursorStyle},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Attribute, Print, SetAttribute},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Normal,
    Insert,
    Command,
}

struct Editor {
    lines: Vec<String>,
    cx: usize, // cursor column (in chars)
    cy: usize, // cursor row
    row_off: usize,
    col_off: usize,
    mode: Mode,
    cmd: String,
    pending: Option<char>, // for dd / gg
    file: Option<PathBuf>,
    dirty: bool,
    quit_armed: bool, // Ctrl+Q twice to quit with unsaved changes
    msg: String,
    quit: bool,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    insert_snapped: bool, // one undo step per insert session
    cmd_kind: char,       // ':' or '/'
    last_search: String,
}

type Snap = (Vec<String>, usize, usize);
const UNDO_LIMIT: usize = 500;

/// Restores the terminal even if we panic.
struct TermGuard;
impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), SetCursorStyle::DefaultUserShape, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn byte_idx(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

impl Editor {
    fn open(path: Option<PathBuf>) -> Self {
        let mut lines: Vec<String> = Vec::new();
        if let Some(p) = &path {
            if let Ok(text) = fs::read_to_string(p) {
                lines = text.lines().map(|l| l.to_string()).collect();
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        Editor {
            lines,
            cx: 0,
            cy: 0,
            row_off: 0,
            col_off: 0,
            mode: Mode::Normal,
            cmd: String::new(),
            pending: None,
            file: path,
            dirty: false,
            quit_armed: false,
            msg: "IV — vi, but backwards (and simple) | i insert · Ctrl+S save · Ctrl+Q quit".into(),
            quit: false,
            undo: Vec::new(),
            redo: Vec::new(),
            insert_snapped: false,
            cmd_kind: ':',
            last_search: String::new(),
        }
    }

    // ---------- undo / redo ----------
    fn snapshot(&mut self) {
        self.undo.push((self.lines.clone(), self.cx, self.cy));
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self) {
        // skip snapshots that match the current text (no-op changes)
        while let Some((lines, cx, cy)) = self.undo.pop() {
            if lines != self.lines {
                self.redo.push((std::mem::replace(&mut self.lines, lines), self.cx, self.cy));
                self.cx = cx;
                self.cy = cy;
                self.dirty = true;
                self.insert_snapped = false;
                self.msg = format!("undo ⏪ ({} left)", self.undo.len());
                return;
            }
        }
        self.msg = "nothing to undo 🤷".into();
    }

    fn redo(&mut self) {
        if let Some((lines, cx, cy)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.lines, lines), self.cx, self.cy));
            self.cx = cx;
            self.cy = cy;
            self.dirty = true;
            self.insert_snapped = false;
            self.msg = "redo ⏩".into();
        } else {
            self.msg = "nothing to redo 🤷".into();
        }
    }

    fn insert_snap(&mut self) {
        if !self.insert_snapped {
            self.snapshot();
            self.insert_snapped = true;
        }
    }

    fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        self.insert_snapped = false;
    }

    // ---------- search ----------
    fn search(&mut self, forward: bool) {
        if self.last_search.is_empty() {
            self.msg = "no search yet, use /something".into();
            return;
        }
        let pat = self.last_search.clone();
        let mut hits: Vec<(usize, usize)> = Vec::new();
        for (y, line) in self.lines.iter().enumerate() {
            for (b, _) in line.match_indices(pat.as_str()) {
                hits.push((y, line[..b].chars().count()));
            }
        }
        if hits.is_empty() {
            self.msg = format!("not found: {pat} 😔");
            return;
        }
        let here = (self.cy, self.cx);
        let found = if forward {
            hits.iter().find(|&&h| h > here).map(|&h| (h, false))
                .unwrap_or((hits[0], true))
        } else {
            hits.iter().rev().find(|&&h| h < here).map(|&h| (h, false))
                .unwrap_or((*hits.last().unwrap(), true))
        };
        let ((y, x), wrapped) = found;
        self.cy = y;
        self.cx = x;
        self.msg = format!("/{pat}{}", if wrapped { "  (wrapped around 🔁)" } else { "" });
    }

    fn len(&self, y: usize) -> usize {
        self.lines[y].chars().count()
    }

    fn clamp(&mut self) {
        self.cy = self.cy.min(self.lines.len() - 1);
        let len = self.len(self.cy);
        let max = if self.mode == Mode::Insert { len } else { len.saturating_sub(1) };
        self.cx = self.cx.min(max);
    }

    fn save(&mut self) {
        let Some(p) = &self.file else {
            self.msg = "no file name 😭 use :w <name>".into();
            return;
        };
        let mut out = self.lines.join("\n");
        out.push('\n');
        match fs::write(p, out) {
            Ok(_) => {
                self.dirty = false;
                self.msg = format!("saved {} ✅", p.display());
            }
            Err(e) => self.msg = format!("save failed: {e}"),
        }
    }

    fn try_quit(&mut self, force: bool) {
        if self.dirty && !force {
            self.msg = "unsaved changes 👀 :q! to yeet them, or :wq".into();
        } else {
            self.quit = true;
        }
    }

    // ---------- editing ----------
    fn insert_char(&mut self, c: char) {
        let b = byte_idx(&self.lines[self.cy], self.cx);
        self.lines[self.cy].insert(b, c);
        self.cx += 1;
        self.dirty = true;
    }

    fn newline(&mut self) {
        let b = byte_idx(&self.lines[self.cy], self.cx);
        let rest = self.lines[self.cy].split_off(b);
        self.lines.insert(self.cy + 1, rest);
        self.cy += 1;
        self.cx = 0;
        self.dirty = true;
    }

    fn backspace(&mut self) {
        if self.cx > 0 {
            let b = byte_idx(&self.lines[self.cy], self.cx - 1);
            self.lines[self.cy].remove(b);
            self.cx -= 1;
            self.dirty = true;
        } else if self.cy > 0 {
            let line = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.len(self.cy);
            self.lines[self.cy].push_str(&line);
            self.dirty = true;
        }
    }

    fn delete_char(&mut self) {
        if self.len(self.cy) > 0 {
            let b = byte_idx(&self.lines[self.cy], self.cx);
            self.lines[self.cy].remove(b);
            self.dirty = true;
        }
    }

    fn delete_line(&mut self) {
        if self.lines.len() == 1 {
            self.lines[0].clear();
        } else {
            self.lines.remove(self.cy);
        }
        self.dirty = true;
    }

    // ---------- keys ----------
    fn handle(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        // global shortcuts for normal people
        if ctrl && k.code == KeyCode::Char('s') {
            self.save();
            return;
        }
        if ctrl && k.code == KeyCode::Char('q') {
            if self.dirty && !self.quit_armed {
                self.quit_armed = true;
                self.msg = "unsaved changes 👀 Ctrl+Q again to quit anyway".into();
            } else {
                self.quit = true;
            }
            return;
        }
        self.quit_armed = false;
        if ctrl && k.code == KeyCode::Char('z') {
            self.undo();
            self.clamp();
            return;
        }
        if ctrl && k.code == KeyCode::Char('y') {
            self.redo();
            self.clamp();
            return;
        }

        match self.mode {
            Mode::Normal => self.normal(k),
            Mode::Insert => self.insert(k),
            Mode::Command => self.command(k),
        }
        self.clamp();
    }

    fn normal(&mut self, k: KeyEvent) {
        if let Some(p) = self.pending.take() {
            match (p, k.code) {
                ('d', KeyCode::Char('d')) => {
                    self.snapshot();
                    self.delete_line();
                }
                ('g', KeyCode::Char('g')) => self.cy = 0,
                _ => {}
            }
            return;
        }
        match k.code {
            KeyCode::Char('h') | KeyCode::Left => self.cx = self.cx.saturating_sub(1),
            KeyCode::Char('l') | KeyCode::Right => self.cx += 1,
            KeyCode::Char('j') | KeyCode::Down => self.cy += 1,
            KeyCode::Char('k') | KeyCode::Up => self.cy = self.cy.saturating_sub(1),
            KeyCode::Char('0') | KeyCode::Home => self.cx = 0,
            KeyCode::Char('$') | KeyCode::End => self.cx = usize::MAX,
            KeyCode::Char('G') => self.cy = self.lines.len() - 1,
            KeyCode::Char('i') => self.enter_insert(),
            KeyCode::Char('I') => {
                self.cx = 0;
                self.enter_insert();
            }
            KeyCode::Char('a') => {
                self.enter_insert();
                if self.len(self.cy) > 0 {
                    self.cx += 1;
                }
            }
            KeyCode::Char('A') => {
                self.enter_insert();
                self.cx = self.len(self.cy);
            }
            KeyCode::Char('o') => {
                self.snapshot();
                self.lines.insert(self.cy + 1, String::new());
                self.cy += 1;
                self.cx = 0;
                self.dirty = true;
                self.enter_insert();
                self.insert_snapped = true; // the o + typing is one undo step
            }
            KeyCode::Char('O') => {
                self.snapshot();
                self.lines.insert(self.cy, String::new());
                self.cx = 0;
                self.dirty = true;
                self.enter_insert();
                self.insert_snapped = true;
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                self.snapshot();
                self.delete_char();
            }
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => self.redo(),
            KeyCode::Char('n') => self.search(true),
            KeyCode::Char('N') => self.search(false),
            KeyCode::Char(c @ ('d' | 'g')) => self.pending = Some(c),
            KeyCode::Char(c @ (':' | '/')) => {
                self.cmd.clear();
                self.cmd_kind = c;
                self.mode = Mode::Command;
            }
            _ => {}
        }
    }

    fn insert(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.cx = self.cx.saturating_sub(1);
            }
            KeyCode::Enter => {
                self.insert_snap();
                self.newline();
            }
            KeyCode::Backspace => {
                self.insert_snap();
                self.backspace();
            }
            KeyCode::Delete => {
                self.insert_snap();
                self.delete_char();
            }
            KeyCode::Tab => {
                self.insert_snap();
                (0..4).for_each(|_| self.insert_char(' '));
            }
            KeyCode::Left => self.cx = self.cx.saturating_sub(1),
            KeyCode::Right => self.cx += 1,
            KeyCode::Up => self.cy = self.cy.saturating_sub(1),
            KeyCode::Down => self.cy += 1,
            KeyCode::Home => self.cx = 0,
            KeyCode::End => self.cx = usize::MAX,
            KeyCode::Char(c) => {
                self.insert_snap();
                self.insert_char(c);
            }
            _ => {}
        }
    }

    fn command(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                if self.cmd.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                let cmd = std::mem::take(&mut self.cmd);
                if self.cmd_kind == '/' {
                    if !cmd.is_empty() {
                        self.last_search = cmd;
                    }
                    self.search(true);
                } else {
                    self.run(cmd.trim());
                }
            }
            KeyCode::Char(c) => self.cmd.push(c),
            _ => {}
        }
    }

    fn run(&mut self, cmd: &str) {
        let (name, arg) = match cmd.split_once(' ') {
            Some((n, a)) => (n, Some(a.trim())),
            None => (cmd, None),
        };
        if let Some(a) = arg.filter(|a| !a.is_empty()) {
            if matches!(name, "w" | "wq" | "x") {
                self.file = Some(PathBuf::from(a));
            }
        }
        match name {
            "w" => self.save(),
            "wq" | "x" | "Wq" | "WQ" => {
                self.save();
                if !self.dirty {
                    self.quit = true;
                }
            }
            // typos still quit, as promised 😭
            "q" | "Q" | "qa" | "qw" | "quit" | "exit" => self.try_quit(false),
            "q!" | "Q!" | "qa!" => self.try_quit(true),
            "" => {}
            n if n.chars().all(|c| c.is_ascii_digit()) => {
                self.cy = n.parse::<usize>().unwrap_or(1).saturating_sub(1);
            }
            other => self.msg = format!("unknown command: {other} 🤨"),
        }
    }

    // ---------- drawing ----------
    fn render(&mut self, out: &mut Stdout) -> io::Result<()> {
        let (w, h) = terminal::size()?;
        let (w, h) = (w as usize, h as usize);
        let text_rows = h.saturating_sub(2).max(1);
        let gutter = self.lines.len().to_string().len() + 1;
        let text_cols = w.saturating_sub(gutter).max(1);

        // scroll to keep cursor visible
        if self.cy < self.row_off {
            self.row_off = self.cy;
        }
        if self.cy >= self.row_off + text_rows {
            self.row_off = self.cy + 1 - text_rows;
        }
        if self.cx < self.col_off {
            self.col_off = self.cx;
        }
        if self.cx >= self.col_off + text_cols {
            self.col_off = self.cx + 1 - text_cols;
        }

        queue!(out, MoveTo(0, 0))?;
        for r in 0..text_rows {
            let y = self.row_off + r;
            queue!(out, MoveTo(0, r as u16), Clear(ClearType::CurrentLine))?;
            if y < self.lines.len() {
                let num = format!("{:>width$} ", y + 1, width = gutter - 1);
                let visible: String =
                    self.lines[y].chars().skip(self.col_off).take(text_cols).collect();
                queue!(
                    out,
                    SetAttribute(Attribute::Dim),
                    Print(num),
                    SetAttribute(Attribute::Reset),
                    Print(visible)
                )?;
            } else {
                queue!(out, SetAttribute(Attribute::Dim), Print("~"), SetAttribute(Attribute::Reset))?;
            }
        }

        // status bar
        let mode = match self.mode {
            Mode::Normal => " NORMAL ",
            Mode::Insert => " INSERT ",
            Mode::Command => " COMMAND ",
        };
        let name = self
            .file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "[no name]".into());
        let left = format!("{mode} {name}{}", if self.dirty { " [+]" } else { "" });
        let right = format!("{}:{} ", self.cy + 1, self.cx + 1);
        let pad = w.saturating_sub(left.chars().count() + right.chars().count());
        let bar: String = format!("{left}{}{right}", " ".repeat(pad)).chars().take(w).collect();
        queue!(
            out,
            MoveTo(0, text_rows as u16),
            SetAttribute(Attribute::Reverse),
            Print(bar),
            SetAttribute(Attribute::Reset),
            MoveTo(0, text_rows as u16 + 1),
            Clear(ClearType::CurrentLine)
        )?;

        // command / message line + cursor
        if self.mode == Mode::Command {
            queue!(out, Print(format!("{}{}", self.cmd_kind, self.cmd)))?;
            queue!(out, MoveTo(self.cmd.chars().count() as u16 + 1, text_rows as u16 + 1))?;
        } else {
            let msg: String = self.msg.chars().take(w).collect();
            queue!(out, Print(msg))?;
            let shape = if self.mode == Mode::Insert {
                SetCursorStyle::SteadyBar
            } else {
                SetCursorStyle::SteadyBlock
            };
            queue!(
                out,
                shape,
                MoveTo(
                    (gutter + self.cx - self.col_off) as u16,
                    (self.cy - self.row_off) as u16
                )
            )?;
        }
        out.flush()
    }
}

fn main() -> io::Result<()> {
    let arg = env::args().nth(1);
    if matches!(arg.as_deref(), Some("--version" | "-v")) {
        println!("IV {} — vi, but backwards (and simple)", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut ed = Editor::open(arg.map(PathBuf::from));
    let mut out = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(out, EnterAlternateScreen)?;
    let _guard = TermGuard;

    while !ed.quit {
        ed.render(&mut out)?;
        match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                if ed.mode != Mode::Command {
                    // clear old message on next keypress (unless it's a global shortcut)
                    ed.msg.clear();
                }
                ed.handle(k);
            }
            _ => {} // resize etc → just redraw
        }
    }
    Ok(())
}
