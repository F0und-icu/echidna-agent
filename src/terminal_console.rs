//! One Chat-scoped terminal draft and presentation owner. No persistent input.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::{cursor, execute, terminal};
use tokio::sync::Notify;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

static ACTIVE: Mutex<Weak<Mutex<Console>>> = Mutex::new(Weak::new());

pub(crate) struct Terminal {
    console: Arc<Mutex<Console>>,
    pub(crate) wake: Arc<Notify>,
    previews: mpsc::Receiver<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputMode {
    Task,
    Processing,
    Paste,
}

impl InputMode {
    pub(crate) fn prompt(self) -> &'static str {
        match self {
            Self::Task => "you> ",
            Self::Processing => "working> ",
            Self::Paste => "...> ",
        }
    }
}

#[derive(Clone)]
pub(crate) struct PreviewSender {
    sender: mpsc::SyncSender<String>,
    dropped: Arc<AtomicU64>,
    wake: Arc<Notify>,
}

impl PreviewSender {
    pub(crate) fn send(&self, frame: String) {
        if self.sender.try_send(frame).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.wake.notify_one();
    }
}

struct Console {
    draft: String,
    cursor: usize,
    start: usize,
    columns: usize,
    mode: InputMode,
    // `columns` retains a pending wrap already advanced to reserve the draft row.
    output_column: usize,
    // A retained output row may have its cursor at zero after a bare CR.
    output_above: bool,
    preview: PreviewSender,
}

impl Terminal {
    pub(crate) fn start() -> io::Result<Option<Self>> {
        if !io::stdin().is_terminal()
            || !io::stdout().is_terminal()
            || std::env::var_os("TERM").is_some_and(|term| term == "dumb")
        {
            return Ok(None);
        }
        let (columns, _) = terminal::size()?;
        let (sender, previews) = mpsc::sync_channel(8);
        let wake = Arc::new(Notify::new());
        let console = Arc::new(Mutex::new(Console {
            draft: String::new(),
            cursor: 0,
            start: 0,
            columns: usize::from(columns.max(2)),
            mode: InputMode::Task,
            output_column: 0,
            output_above: false,
            preview: PreviewSender {
                sender,
                dropped: Arc::new(AtomicU64::new(0)),
                wake: Arc::clone(&wake),
            },
        }));
        terminal::enable_raw_mode()?;
        let terminal = Self {
            console,
            wake,
            previews,
        };
        execute!(io::stdout(), EnableBracketedPaste)?;
        *ACTIVE.lock().map_err(poisoned)? = Arc::downgrade(&terminal.console);
        terminal.console.lock().map_err(poisoned)?.redraw()?;
        Ok(Some(terminal))
    }

    pub(crate) fn mode(&self, mode: InputMode) -> io::Result<()> {
        let mut state = self.console.lock().map_err(poisoned)?;
        state.mode = mode;
        state.redraw()
    }

    pub(crate) fn edit(&self, edit: Edit) -> io::Result<Option<String>> {
        let mut state = self.console.lock().map_err(poisoned)?;
        match edit {
            Edit::Insert(text) => {
                let at = state.cursor;
                state.draft.insert_str(at, &text);
                state.cursor += text.len();
                state.align_grapheme_boundaries();
            }
            Edit::Left => state.cursor = state.previous(),
            Edit::Right => state.cursor = state.next(),
            Edit::Home => state.cursor = 0,
            Edit::End => state.cursor = state.draft.len(),
            Edit::Backspace => {
                let previous = state.previous();
                let cursor = state.cursor;
                state.draft.replace_range(previous..cursor, "");
                state.cursor = previous;
                state.align_grapheme_boundaries();
            }
            Edit::Delete => {
                let next = state.next();
                let cursor = state.cursor;
                state.draft.replace_range(cursor..next, "");
                state.align_grapheme_boundaries();
            }
            Edit::Submit => {
                let line = std::mem::take(&mut state.draft);
                state.cursor = 0;
                state.start = 0;
                if state.mode != InputMode::Paste && line.trim().is_empty() {
                    state.redraw()?;
                    return Ok(None);
                }
                // Echo accepted text; only the returned original String owns
                // submission. Paste whitespace is visible, never executed.
                write!(
                    io::stdout(),
                    "\r\x1b[2K{}{}\r\n",
                    state.mode.prompt(),
                    draft_display(&line)
                )?;
                state.output_column = 0;
                state.output_above = false;
                state.redraw()?;
                return Ok(Some(line));
            }
            Edit::Resize(columns) => {
                state.columns = usize::from(columns.max(2));
                state.output_column %= state.columns;
            }
            Edit::Clear => {
                state.draft.clear();
                state.cursor = 0;
                state.start = 0;
            }
        }
        state.redraw()?;
        Ok(None)
    }

    pub(crate) fn empty(&self) -> io::Result<bool> {
        Ok(self.console.lock().map_err(poisoned)?.draft.is_empty())
    }

    pub(crate) fn drain_previews(&self) -> io::Result<()> {
        let mut state = self.console.lock().map_err(poisoned)?;
        let dropped = state.preview.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            state.output(
                format!("[工具展示拥塞：跳过 {dropped} 条预览；执行事实以 Session 记录为准]\n")
                    .as_bytes(),
            )?;
        }
        for frame in self.previews.try_iter() {
            state.output(frame.as_bytes())?;
        }
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE.lock() {
            *active = Weak::new();
        }
        let _ = execute!(
            io::stdout(),
            cursor::MoveToColumn(0),
            terminal::Clear(terminal::ClearType::CurrentLine),
            DisableBracketedPaste
        );
        let _ = terminal::disable_raw_mode();
    }
}

pub(crate) enum Edit {
    Insert(String),
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Submit,
    Resize(u16),
    Clear,
}

impl Console {
    fn align_grapheme_boundaries(&mut self) {
        // Insertion or deletion can join adjacent graphemes, including ZWJ
        // and regional-indicator pairs. Neither cursor nor viewport may split one.
        self.cursor = self
            .draft
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .find(|at| *at >= self.cursor)
            .unwrap_or(self.draft.len());
        self.start = self
            .draft
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .take_while(|at| *at <= self.start.min(self.cursor))
            .last()
            .unwrap_or(0);
    }

    fn previous(&self) -> usize {
        self.draft[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(at, _)| at)
    }
    fn next(&self) -> usize {
        self.draft[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len())
    }
    fn redraw(&mut self) -> io::Result<()> {
        let prompt = if self.mode.prompt().width() + 1 < self.columns {
            self.mode.prompt()
        } else if self.mode == InputMode::Processing {
            "*"
        } else {
            ">"
        };
        let capacity = self.columns.saturating_sub(prompt.width() + 1);
        self.start = self.start.min(self.cursor);
        let mut cursor_cells = draft_display(&self.draft[self.start..self.cursor]).width();
        for grapheme in self.draft[self.start..self.cursor].graphemes(true) {
            if cursor_cells <= capacity {
                break;
            }
            cursor_cells -= draft_display(grapheme).width();
            self.start += grapheme.len();
        }
        let mut end = self.start;
        let mut cells = 0;
        for grapheme in self.draft[self.start..].graphemes(true) {
            let width = draft_display(grapheme).width();
            if cells + width > capacity {
                break;
            }
            cells += width;
            end += grapheme.len();
        }
        let column = prompt.width() + cursor_cells;
        let mut stdout = io::stdout().lock();
        write!(
            stdout,
            "\r\x1b[2K{prompt}{}\r",
            draft_display(&self.draft[self.start..end])
        )?;
        execute!(stdout, cursor::MoveToColumn(column as u16))?;
        stdout.flush()
    }
    fn output(&mut self, bytes: &[u8]) -> io::Result<()> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut stdout = io::stdout().lock();
        write!(stdout, "\r\x1b[2K")?;
        if self.output_above {
            execute!(
                stdout,
                cursor::MoveUp(1),
                cursor::MoveToColumn(self.output_column as u16)
            )?;
        }
        // This frame is transient, like the previous newline translation; it
        // never accumulates output between calls. A reserved full-row advance
        // must be consumed by a later LF, or undone for a CR/HT on that row.
        let mut rendered = String::with_capacity(text.len());
        let mut reserved_wrap = self.output_column >= self.columns;
        let mut escape = false;
        for grapheme in text.graphemes(true) {
            if grapheme == "\x1b" {
                rendered.push_str(grapheme);
                escape = true;
                continue;
            }
            if escape {
                rendered.push_str(grapheme);
                if grapheme != "[" && grapheme.chars().all(|c| ('@'..='~').contains(&c)) {
                    escape = false;
                }
                continue;
            }
            if reserved_wrap {
                reserved_wrap = false;
                match grapheme {
                    "\n" | "\r\n" => {
                        self.output_column = 0;
                        self.output_above = false;
                        continue;
                    }
                    "\r" | "\t" => {
                        let column = if grapheme == "\r" {
                            0
                        } else {
                            self.columns - 1
                        };
                        rendered.push_str(&format!("\x1b[1A\x1b[{}G", column + 1));
                        self.output_column = column;
                        self.output_above = true;
                    }
                    _ => {}
                }
            }
            match grapheme {
                "\n" | "\r\n" => {
                    // Raw mode disables the canonical terminal's LF -> CRLF.
                    if grapheme == "\r\n" {
                        rendered.push('\r');
                    }
                    rendered.push_str("\r\n");
                    self.output_column = 0;
                    self.output_above = false;
                }
                "\r" => {
                    rendered.push('\r');
                    self.output_column = 0;
                }
                "\t" => {
                    rendered.push('\t');
                    self.output_column =
                        (self.output_column + 8 - self.output_column % 8).min(self.columns - 1);
                    self.output_above = true;
                }
                _ => {
                    rendered.push_str(grapheme);
                    let width = grapheme.width();
                    if self.output_column + width > self.columns {
                        self.output_column = 0;
                    }
                    self.output_column += width;
                    self.output_above = true;
                }
            }
        }
        stdout.write_all(rendered.as_bytes())?;
        if self.output_above {
            stdout.write_all(b"\r\n")?;
        }
        if self.output_column >= self.columns {
            self.output_above = false;
        }
        drop(stdout);
        self.redraw()
    }
}

fn draft_display(text: &str) -> String {
    text.replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> io::Error {
    io::Error::other("terminal console lock poisoned")
}

fn active() -> io::Result<Option<Arc<Mutex<Console>>>> {
    Ok(ACTIVE.lock().map_err(poisoned)?.upgrade())
}

pub(crate) fn preview_sender() -> Option<PreviewSender> {
    if !io::stderr().is_terminal() {
        return None;
    }
    active()
        .expect("terminal console lock poisoned")
        .map(|state| {
            state
                .lock()
                .expect("terminal console lock poisoned")
                .preview
                .clone()
        })
}

pub(crate) struct Output {
    stderr: bool,
}
pub(crate) fn stdout() -> Output {
    Output { stderr: false }
}
pub(crate) fn stderr() -> Output {
    Output { stderr: true }
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if (!self.stderr || io::stderr().is_terminal())
            && let Some(state) = active()?
        {
            state.lock().map_err(poisoned)?.output(bytes)?;
        } else if self.stderr {
            io::stderr().write_all(bytes)?;
        } else {
            io::stdout().write_all(bytes)?;
        }
        Ok(bytes.len())
    }
    fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> io::Result<()> {
        self.write_all(args.to_string().as_bytes())
    }
    fn flush(&mut self) -> io::Result<()> {
        if (!self.stderr || io::stderr().is_terminal()) && active()?.is_some() {
            io::stdout().flush()
        } else if self.stderr {
            io::stderr().flush()
        } else {
            io::stdout().flush()
        }
    }
}
