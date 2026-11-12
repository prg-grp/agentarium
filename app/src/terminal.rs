//! The VM consoles: the session in the Terminal tab and the read-only boot log in the
//! Logs tab.
//!
//! `alacritty_terminal` reads a console, parses its output on its own I/O
//! thread, and answers terminal queries. This module forwards input, resizes,
//! and scrolling to it, and sends the visible screen to the UI as rows of
//! styled runs (`TermLine`), replacing only rows that changed.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use alacritty_terminal::event::{Event, EventListener, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{Config, Term, TermMode, point_to_viewport, viewport_to_point};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Rgb};
use polling::{Event as PollEvent, PollMode, Poller};
use slint::platform::Key;
use slint::{Color, ComponentHandle, Model, ModelRc, VecModel, Weak};

use crate::{AppWindow, TermLine, TermRun, TermSpan, TerminalSession};

/// alacritty's own maximum; history is allocated as it fills.
const SCROLLBACK: usize = 100_000;
const DEFAULT_FG: (u8, u8, u8) = (0xD6, 0xDC, 0xE5);
const WELL_BG: (u8, u8, u8) = (0x0C, 0x0E, 0x12);

thread_local! {
    /// Runs of characters a URL may contain, as in Alacritty's URL hint. Links start at
    /// an http(s) scheme within them.
    static URL_CHARS: RefCell<Option<RegexSearch>> = RefCell::new(
        RegexSearch::new(r#"[^\x00-\x1f\x7f-\x9f<>"\s{-}\^⟨⟩`\\]+"#).ok(),
    );
}

/// Normal and bright ANSI colors, tuned to the app theme.
const ANSI: [(u8, u8, u8); 16] = [
    (0x1B, 0x1F, 0x27),
    (0xF0, 0x8C, 0x92),
    (0x78, 0xC9, 0x9A),
    (0xE8, 0xBC, 0x6A),
    (0x8B, 0xA7, 0xFF),
    (0xC9, 0xA2, 0xF5),
    (0x7C, 0xCF, 0xD6),
    (0xD6, 0xDC, 0xE5),
    (0x5A, 0x64, 0x75),
    (0xF7, 0xB0, 0xB4),
    (0x9E, 0xDD, 0xB8),
    (0xF2, 0xD1, 0x96),
    (0xB3, 0xC6, 0xFF),
    (0xDC, 0xC3, 0xF9),
    (0xA6, 0xE1, 0xE6),
    (0xF0, 0xF3, 0xF7),
];

/// Keys that map to fixed byte sequences. Arrows are handled separately
/// because they depend on the application cursor mode.
const KEY_SEQUENCES: &[(Key, &str)] = &[
    (Key::Return, "\r"),
    (Key::Backspace, "\x7f"),
    (Key::Escape, "\x1b"),
    (Key::Backtab, "\x1b[Z"),
    (Key::Delete, "\x1b[3~"),
    (Key::Insert, "\x1b[2~"),
    (Key::Home, "\x1b[H"),
    (Key::End, "\x1b[F"),
    (Key::PageUp, "\x1b[5~"),
    (Key::PageDown, "\x1b[6~"),
    (Key::F1, "\x1bOP"),
    (Key::F2, "\x1bOQ"),
    (Key::F3, "\x1bOR"),
    (Key::F4, "\x1bOS"),
    (Key::F5, "\x1b[15~"),
    (Key::F6, "\x1b[17~"),
    (Key::F7, "\x1b[18~"),
    (Key::F8, "\x1b[19~"),
    (Key::F9, "\x1b[20~"),
    (Key::F10, "\x1b[21~"),
    (Key::F11, "\x1b[23~"),
    (Key::F12, "\x1b[24~"),
];

/// Cell attributes that affect drawing.
const STYLE_FLAGS: Flags = Flags::BOLD
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::DIM)
    .union(Flags::INVERSE);

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: AnsiColor,
    bg: AnsiColor,
    flags: Flags,
}

#[derive(Clone, PartialEq)]
struct Run {
    col: usize,
    width: usize,
    text: String,
    style: Style,
}

/// Connects one shell session's event loop back to the UI.
struct Link {
    ui: Weak<AppWindow>,
    term: OnceLock<std::sync::Weak<FairMutex<Term<Listener>>>>,
    sender: OnceLock<EventLoopSender>,
    /// Runs per row as last sent to the UI, to update only changed rows.
    previous: Mutex<Vec<Vec<Run>>>,
    redraw_pending: AtomicBool,
    /// Cleared when the session is replaced, so its late events are ignored.
    active: AtomicBool,
    /// Whether the terminal is the one shown in the UI; only that one draws.
    shown: AtomicBool,
    /// Never writes to the console, not even replies to terminal queries.
    read_only: bool,
    /// Pointer position in cell units, to underline the link under it.
    hover: Mutex<Option<(f32, f32)>>,
}

impl Link {
    fn write(&self, bytes: Vec<u8>) {
        if !self.read_only
            && let Some(sender) = self.sender.get()
        {
            let _ = sender.send(Msg::Input(Cow::Owned(bytes)));
        }
    }
}

#[derive(Clone)]
struct Listener(Arc<Link>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let link = &self.0;
        if !link.active.load(Ordering::Acquire) {
            return;
        }
        match event {
            Event::Wakeup => {
                // Schedule at most one redraw at a time, so bursts of output
                // don't flood the event loop. A hidden terminal redraws when shown.
                if link.shown.load(Ordering::Acquire)
                    && !link.redraw_pending.swap(true, Ordering::AcqRel)
                {
                    let link = link.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if !link.active.load(Ordering::Acquire)
                            || !link.shown.load(Ordering::Acquire)
                        {
                            link.redraw_pending.store(false, Ordering::Release);
                            return;
                        }
                        let term = link.term.get().and_then(|term| term.upgrade());
                        if let (Some(ui), Some(term)) = (link.ui.upgrade(), term) {
                            redraw(&link, &term.lock(), &ui);
                        }
                    });
                }
            }
            // Replies to terminal queries, e.g. cursor position or colors.
            Event::PtyWrite(text) => link.write(text.into_bytes()),
            // The terminal is locked while this is sent, so answer from the
            // default palette instead of locking it again.
            Event::ColorRequest(index, format) if !link.read_only => {
                let (r, g, b) = palette(index);
                link.write(format(Rgb { r, g, b }).into_bytes());
            }
            Event::ClipboardStore(_, text) => {
                let _ = slint::invoke_from_event_loop(move || {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
                });
            }
            _ => {}
        }
    }
}

/// Screen size for `alacritty_terminal`, as (lines, columns).
struct ScreenSize(usize, usize);

impl Dimensions for ScreenSize {
    fn total_lines(&self) -> usize {
        self.0
    }

    fn screen_lines(&self) -> usize {
        self.0
    }

    fn columns(&self) -> usize {
        self.1
    }
}

struct Session {
    term: Arc<FairMutex<Term<Listener>>>,
    link: Arc<Link>,
}

/// One instance's console. Several can exist; only the shown one draws into the
/// `TerminalSession` global, the others keep their output until shown.
pub struct Terminal {
    ui: Weak<AppWindow>,
    /// Screen size as (lines, columns).
    size: Cell<(usize, usize)>,
    session: RefCell<Option<Session>>,
    shown: Cell<bool>,
    /// The exit bar text while no session runs.
    exit_message: RefCell<Option<String>>,
    /// Only shows output: input is ignored, while selecting and copying still work.
    read_only: bool,
}

impl Terminal {
    /// Creates a hidden terminal without a session; `message` is shown until one is attached.
    pub fn new(ui: Weak<AppWindow>, message: &str, read_only: bool) -> Terminal {
        Terminal {
            ui,
            size: Cell::new((24, 80)),
            session: RefCell::new(None),
            shown: Cell::new(false),
            exit_message: RefCell::new(Some(message.to_owned())),
            read_only,
        }
    }

    /// Makes this terminal the one drawn in the UI, with its output and exit bar.
    pub fn show(&self) {
        self.set_shown(true);
        if let Some(ui) = self.ui.upgrade() {
            let session = ui.global::<TerminalSession>();
            session.set_lines(ModelRc::new(VecModel::<TermLine>::default()));
            session.set_selection(ModelRc::default());
            session.set_link(ModelRc::default());
            session.set_scroll_offset(0);
            session.set_history_size(0);
            let message = self.exit_message.borrow();
            session.set_exited(message.is_some());
            session.set_exit_message(message.as_deref().unwrap_or_default().into());
        }
        self.redraw();
    }

    /// Stops drawing into the UI; output keeps being collected.
    pub fn hide(&self) {
        self.set_shown(false);
    }

    fn set_shown(&self, shown: bool) {
        self.shown.set(shown);
        if let Some(session) = self.session.borrow().as_ref() {
            session.link.shown.store(shown, Ordering::Release);
        }
    }

    /// Shows a new console, replacing the previous session and its output.
    pub fn attach(&self, console: Console) -> io::Result<()> {
        self.stop();
        *self.exit_message.borrow_mut() = None;
        if self.shown.get()
            && let Some(ui) = self.ui.upgrade()
        {
            let session = ui.global::<TerminalSession>();
            session.set_lines(ModelRc::new(VecModel::<TermLine>::default()));
            session.set_exited(false);
        }

        let (lines, cols) = self.size.get();
        let link = Arc::new(Link {
            ui: self.ui.clone(),
            term: OnceLock::new(),
            sender: OnceLock::new(),
            previous: Mutex::new(Vec::new()),
            redraw_pending: AtomicBool::new(false),
            active: AtomicBool::new(true),
            shown: AtomicBool::new(self.shown.get()),
            read_only: self.read_only,
            hover: Mutex::new(None),
        });
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &ScreenSize(lines, cols),
            Listener(link.clone()),
        )));
        let _ = link.term.set(Arc::downgrade(&term));

        let event_loop =
            EventLoop::new(term.clone(), Listener(link.clone()), console, true, false)?;
        let _ = link.sender.set(event_loop.channel());
        // `resize` only passes on changes, so the console learns its first size here.
        let _ = event_loop
            .channel()
            .send(Msg::Resize(window_size(lines, cols)));
        event_loop.spawn();

        *self.session.borrow_mut() = Some(Session { term, link });
        self.redraw();
        Ok(())
    }

    /// Ends the session and shows `message` in the exit bar. Output stays
    /// visible and scrollable until the next `attach`.
    pub fn detach(&self, message: &str) {
        self.stop();
        *self.exit_message.borrow_mut() = Some(message.to_owned());
        if self.shown.get()
            && let Some(ui) = self.ui.upgrade()
        {
            let session = ui.global::<TerminalSession>();
            if session
                .get_lines()
                .as_any()
                .downcast_ref::<VecModel<TermLine>>()
                .is_none()
            {
                session.set_lines(ModelRc::new(VecModel::<TermLine>::default()));
            }
            session.set_exit_message(message.into());
            session.set_exited(true);
        }
    }

    /// Handles a key event from the UI. Returns whether the key was used.
    pub fn key(&self, text: &str, control: bool, alt: bool, meta: bool, shift: bool) -> bool {
        // On macOS, Slint reports Command as `control` and Control as `meta`.
        let (ctrl, cmd) = if cfg!(target_os = "macos") {
            (meta, control)
        } else {
            (control, false)
        };
        let _ = alt; // Option already arrives as the composed character.

        // Selecting and copying also work on the output kept after the session ended.
        if cmd && text.eq_ignore_ascii_case("a") {
            self.with_term(|term| {
                let start = Point::new(term.topmost_line(), Column(0));
                let end = Point::new(term.bottommost_line(), term.last_column());
                let mut selection = Selection::new(SelectionType::Lines, start, Side::Left);
                selection.update(end, Side::Right);
                term.selection = Some(selection);
            });
            self.redraw();
            return true;
        }
        if cmd && text.eq_ignore_ascii_case("c") {
            let Some(text) = self.with_term(|term| term.selection_to_string()).flatten() else {
                return false;
            };
            // Empty rows below the output would otherwise arrive as blank lines.
            let text = text.trim_end_matches('\n').to_owned();
            let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
            return true;
        }
        // Moving through the scrollback, also in the boot log and after the session ended.
        let is = |key: Key| text.chars().eq([char::from(key)]);
        let scroll = if cmd && is(Key::UpArrow) {
            Some(Scroll::Top)
        } else if cmd && is(Key::DownArrow) {
            Some(Scroll::Bottom)
        } else if shift && is(Key::PageUp) {
            Some(Scroll::PageUp)
        } else if shift && is(Key::PageDown) {
            Some(Scroll::PageDown)
        } else {
            None
        };
        if let Some(scroll) = scroll {
            self.scroll_display(scroll);
            return true;
        }

        let running = self
            .session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.link.active.load(Ordering::Acquire));
        if !running || self.read_only {
            return false;
        }

        if cmd {
            if text.eq_ignore_ascii_case("v") {
                self.paste();
                return true;
            }
            return false;
        }

        let Some(mode) = self.with_term(|term| *term.mode()) else {
            return false;
        };
        let Some(bytes) = key_bytes(text, ctrl, shift, mode.contains(TermMode::APP_CURSOR)) else {
            return false;
        };
        self.send(bytes);
        true
    }

    pub fn resize(&self, cols: i32, rows: i32) {
        let (Ok(cols), Ok(lines)) = (usize::try_from(cols), usize::try_from(rows)) else {
            return;
        };
        if cols == 0 || lines == 0 || self.size.get() == (lines, cols) {
            return;
        }
        self.size.set((lines, cols));
        if let Some(session) = self.session.borrow().as_ref() {
            session.term.lock().resize(ScreenSize(lines, cols));
            if let Some(sender) = session.link.sender.get() {
                let _ = sender.send(Msg::Resize(window_size(lines, cols)));
            }
        }
        self.redraw();
    }

    /// Scrolls into older output (positive) or back towards the latest (negative).
    pub fn scroll(&self, lines: i32) {
        self.scroll_display(Scroll::Delta(lines));
    }

    /// Scrolls to `offset` lines back from the latest output, clamped to the history.
    pub fn scroll_to(&self, offset: i32) {
        let Some(current) = self.with_term(|term| term.grid().display_offset()) else {
            return;
        };
        self.scroll_display(Scroll::Delta(offset - current as i32));
    }

    pub fn jump_to_latest(&self) {
        self.scroll_display(Scroll::Bottom);
    }

    fn scroll_display(&self, scroll: Scroll) {
        self.with_term(|term| term.scroll_display(scroll));
        self.redraw();
    }

    /// Starts a mouse selection at a position in cell units. A click without
    /// a drag leaves an empty selection, which clears the previous one.
    pub fn select_start(&self, col: f32, row: f32) {
        self.with_term(|term| {
            let (point, side) = selection_point(term, col, row);
            term.selection = Some(Selection::new(SelectionType::Simple, point, side));
        });
        self.redraw();
    }

    /// Extends the mouse selection to a position in cell units, or starts
    /// one there if there is none (Shift+click without a selection).
    pub fn select_update(&self, col: f32, row: f32) {
        self.with_term(|term| {
            let (point, side) = selection_point(term, col, row);
            match term.selection.as_mut() {
                Some(selection) => selection.update(point, side),
                None => term.selection = Some(Selection::new(SelectionType::Simple, point, side)),
            }
        });
        self.redraw();
    }

    /// Underlines the link under the pointer, given in cell units; `None` when the
    /// pointer left the terminal.
    pub fn hover(&self, at: Option<(f32, f32)>) {
        let at = at.map(|(col, row)| (col.floor(), row.floor()));
        if let Some(session) = self.session.borrow().as_ref() {
            let mut hover = session.link.hover.lock().unwrap();
            if *hover == at {
                return;
            }
            *hover = at;
        }
        self.redraw();
    }

    /// Opens the http(s) link at a position in cell units in the default browser.
    /// Returns whether there was one.
    pub fn open_link(&self, col: f32, row: f32) -> bool {
        let Some(Some((_, url))) =
            self.with_term(|term| link_at(term, selection_point(term, col, row).0))
        else {
            return false;
        };
        // The URL comes from the guest, so only web links are handed to `open`.
        let lower = url.to_ascii_lowercase();
        if !lower.starts_with("http://") && !lower.starts_with("https://") {
            return false;
        }
        if let Err(error) = std::process::Command::new("open").arg(&url).status() {
            eprintln!("Couldn't open {url}: {error}");
        }
        true
    }

    fn paste(&self) {
        let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) else {
            return;
        };
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let bracketed = self
            .with_term(|term| term.mode().contains(TermMode::BRACKETED_PASTE))
            .unwrap_or(false);
        if bracketed {
            self.send(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.send(text.into_bytes());
        }
    }

    fn send(&self, bytes: Vec<u8>) {
        if let Some(session) = self.session.borrow().as_ref() {
            session.link.write(bytes);
        }
        // Typing returns the view to the latest output.
        if self.with_term(|term| term.grid().display_offset()) > Some(0) {
            self.jump_to_latest();
        }
    }

    fn with_term<R>(&self, f: impl FnOnce(&mut Term<Listener>) -> R) -> Option<R> {
        let session = self.session.borrow();
        let session = session.as_ref()?;
        let mut term = session.term.lock();
        Some(f(&mut term))
    }

    fn redraw(&self) {
        if !self.shown.get() {
            return;
        }
        let session = self.session.borrow();
        let (Some(ui), Some(session)) = (self.ui.upgrade(), session.as_ref()) else {
            return;
        };
        redraw(&session.link, &session.term.lock(), &ui);
    }

    /// Shuts down the session's I/O. The terminal and its scrollback are kept
    /// until `attach` replaces them.
    fn stop(&self) {
        if let Some(session) = self.session.borrow().as_ref()
            && session.link.active.swap(false, Ordering::AcqRel)
            && let Some(sender) = session.link.sender.get()
        {
            let _ = sender.send(Msg::Shutdown);
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Host end of one of the VM's serial consoles: two pipes, read and write.
pub struct Console {
    reader: ConsoleReader,
    writer: File,
    /// Passes the terminal's size on to the guest as (columns, lines), if set.
    resized: Option<Box<dyn FnMut(u16, u16) + Send>>,
}

impl Console {
    pub fn new(reader: OwnedFd, writer: OwnedFd) -> io::Result<Console> {
        // The terminal's I/O thread reads and writes until it would block.
        for fd in [&reader, &writer] {
            let raw = fd.as_raw_fd();
            let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Console {
            reader: ConsoleReader {
                pipe: File::from(reader),
                copy: None,
            },
            writer: File::from(writer),
            resized: None,
        })
    }

    /// Calls `resized` with (columns, lines) whenever the terminal changes size. A serial
    /// line can't carry the size itself, so the guest learns it some other way.
    pub fn with_resize(mut self, resized: impl FnMut(u16, u16) + Send + 'static) -> Console {
        self.resized = Some(Box::new(resized));
        self
    }

    /// Also appends everything the guest writes to `file`.
    pub fn with_copy(mut self, file: File) -> Console {
        self.reader.copy = Some(file);
        self
    }
}

/// Reads a console's pipe, and copies what it reads to a file if one is set.
pub struct ConsoleReader {
    pipe: File,
    copy: Option<File>,
}

impl Read for ConsoleReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.pipe.read(buf)?;
        // A failing copy (a full disk, say) is given up, so the terminal keeps running.
        if let Some(copy) = self.copy.as_mut()
            && let Err(error) = copy.write_all(&buf[..read])
        {
            eprintln!("Couldn't write the console's log file: {error}");
            self.copy = None;
        }
        Ok(read)
    }
}

/// Splits the terminal's interest between the two pipes under the same key.
fn split_interest(interest: PollEvent) -> (PollEvent, PollEvent) {
    let mut read = PollEvent::readable(interest.key);
    read.readable = interest.readable;
    let mut write = PollEvent::writable(interest.key);
    write.writable = interest.writable;
    (read, write)
}

impl EventedReadWrite for Console {
    type Reader = ConsoleReader;
    type Writer = File;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        interest: PollEvent,
        mode: PollMode,
    ) -> io::Result<()> {
        let (read, write) = split_interest(interest);
        unsafe {
            poll.add_with_mode(&self.reader.pipe, read, mode)?;
            poll.add_with_mode(&self.writer, write, mode)
        }
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        interest: PollEvent,
        mode: PollMode,
    ) -> io::Result<()> {
        let (read, write) = split_interest(interest);
        poll.modify_with_mode(&self.reader.pipe, read, mode)?;
        poll.modify_with_mode(&self.writer, write, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        poll.delete(&self.reader.pipe)?;
        poll.delete(&self.writer)
    }

    fn reader(&mut self) -> &mut ConsoleReader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        &mut self.writer
    }
}

impl EventedPty for Console {
    // There is no child process; the VM's state is polled instead.
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        None
    }
}

impl OnResize for Console {
    fn on_resize(&mut self, size: WindowSize) {
        if let Some(resized) = self.resized.as_mut() {
            resized(size.num_cols, size.num_lines);
        }
    }
}

fn window_size(lines: usize, cols: usize) -> WindowSize {
    WindowSize {
        num_lines: lines as u16,
        num_cols: cols as u16,
        cell_width: 8,
        cell_height: 16,
    }
}

/// Sends the visible screen to the UI, replacing only rows that changed.
fn redraw(link: &Link, term: &Term<Listener>, ui: &AppWindow) {
    link.redraw_pending.store(false, Ordering::Release);

    let session = ui.global::<TerminalSession>();
    let lines = session.get_lines();
    let Some(model) = lines.as_any().downcast_ref::<VecModel<TermLine>>() else {
        return;
    };

    let content = term.renderable_content();
    let rows = term.screen_lines();
    let mut grid: Vec<Vec<Run>> = vec![Vec::new(); rows];
    for indexed in content.display_iter {
        let Some(point) = point_to_viewport(content.display_offset, indexed.point) else {
            continue;
        };
        let Some(runs) = grid.get_mut(point.line) else {
            continue;
        };
        let cell = indexed.cell;
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            if let Some(last) = runs.last_mut() {
                last.width += 1;
            }
            continue;
        }

        let style = Style {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags & STYLE_FLAGS,
        };
        let mut text = String::new();
        if cell.flags.contains(Flags::HIDDEN) {
            text.push(' ');
        } else {
            text.push(cell.c);
            text.extend(cell.zerowidth().unwrap_or_default());
        }
        match runs.last_mut() {
            Some(last) if last.style == style => {
                last.text.push_str(&text);
                last.width += 1;
            }
            _ => runs.push(Run {
                col: point.column.0,
                width: 1,
                text,
                style,
            }),
        }
    }

    let mut previous = link.previous.lock().unwrap();
    previous.resize(rows, Vec::new());
    while model.row_count() > rows {
        model.remove(model.row_count() - 1);
    }
    for (row, mut runs) in grid.into_iter().enumerate() {
        trim_trailing_blanks(&mut runs);
        if row < model.row_count() && previous[row] == runs {
            continue;
        }
        let line = TermLine {
            runs: ModelRc::new(VecModel::from(
                runs.iter()
                    .map(|run| to_ui(run, content.colors))
                    .collect::<Vec<_>>(),
            )),
        };
        if row < model.row_count() {
            model.set_row_data(row, line);
        } else {
            model.push(line);
        }
        previous[row] = runs;
    }

    let cursor = point_to_viewport(content.display_offset, content.cursor.point);
    let (cursor_row, cursor_col) = cursor.map_or((0, 0), |p| (p.line, p.column.0));
    session.set_cursor_row(cursor_row as i32);
    session.set_cursor_col(cursor_col as i32);
    session.set_cursor_visible(cursor.is_some() && content.cursor.shape != CursorShape::Hidden);
    session.set_scroll_offset(content.display_offset as i32);
    session.set_history_size(term.grid().history_size() as i32);

    let offset = content.display_offset;
    let selection = content.selection.map_or_else(Vec::new, |range| {
        row_spans(term, offset, range.start, range.end)
    });
    session.set_selection(ModelRc::new(VecModel::from(selection)));

    let hover = *link.hover.lock().unwrap();
    let hovered = hover.and_then(|(col, row)| link_at(term, selection_point(term, col, row).0));
    let underline = hovered.map_or_else(Vec::new, |(segments, _)| {
        segments
            .iter()
            .flat_map(|run| row_spans(term, offset, *run.start(), *run.end()))
            .collect()
    });
    session.set_link(ModelRc::new(VecModel::from(underline)));
}

/// Cells from `start` to `end` per visible row; rows in between span the full width.
fn row_spans(
    term: &Term<Listener>,
    display_offset: usize,
    start: Point,
    end: Point,
) -> Vec<TermSpan> {
    let mut spans = Vec::new();
    for row in 0..term.screen_lines() {
        let line = row as i32 - display_offset as i32;
        if line < start.line.0 || line > end.line.0 {
            continue;
        }
        let from = if line == start.line.0 {
            start.column.0
        } else {
            0
        };
        let to = if line == end.line.0 {
            end.column.0
        } else {
            term.columns() - 1
        };
        spans.push(TermSpan {
            row: row as i32,
            col: from as i32,
            width: (to + 1 - from) as i32,
        });
    }
    spans
}

/// The http(s) link covering a grid point, as its cells (one range per line) and its
/// text. Links may wrap across rows, and may be broken across lines by the program
/// itself, as Claude Code does with its sign-in link.
fn link_at(term: &Term<Listener>, point: Point) -> Option<(Vec<Match>, String)> {
    // Where a run's scheme starts, in cells.
    let scheme = |run: &Match| {
        let text = term.bounds_to_string(*run.start(), *run.end());
        let lower = text.to_ascii_lowercase();
        let at = ["http://", "https://"]
            .iter()
            .filter_map(|scheme| lower.find(scheme))
            .min()?;
        Some(text[..at].chars().count())
    };
    let line = url_runs(term, point);
    let run = line.iter().find(|run| run.contains(&point))?.clone();
    let mut segments = VecDeque::from([run]);

    // Follow the run up to the line holding its scheme and down to where it stops.
    let mut above = line.clone();
    while segments.len() < 16 && scheme(&segments[0]).is_none() && above.first() == segments.front()
    {
        let top = term.line_search_left(*segments[0].start()).line;
        if top <= term.topmost_line() {
            break;
        }
        let upper = url_runs(term, Point::new(top - 1, Column(0)));
        if !continues(term, &upper, &above) {
            break;
        }
        segments.push_front(upper[upper.len() - 1].clone());
        above = upper;
    }
    let mut below = line;
    while segments.len() < 32 && below.last() == segments.back() {
        let bottom = term.line_search_right(*below[0].end()).line;
        if bottom >= term.bottommost_line() {
            break;
        }
        let lower = url_runs(term, Point::new(bottom + 1, Column(0)));
        if !continues(term, &below, &lower) || scheme(&lower[0]).is_some() {
            break;
        }
        segments.push_back(lower[0].clone());
        below = lower;
    }

    let mut segments = Vec::from(segments);
    let at = scheme(&segments[0])?;
    segments[0] = segments[0].start().add(term, Boundary::Grid, at)..=*segments[0].end();
    let mut url: String = segments
        .iter()
        .map(|run| term.bounds_to_string(*run.start(), *run.end()))
        .collect();
    // Punctuation after a link usually belongs to the sentence, as does a closing
    // bracket without an opening one in the link.
    let mut trimmed = 0;
    while let Some(last) = url.chars().last() {
        let trailing = match last {
            ')' => url.matches('(').count() < url.matches(')').count(),
            ']' => url.matches('[').count() < url.matches(']').count(),
            _ => ".,:;!?'".contains(last),
        };
        if !trailing {
            break;
        }
        url.pop();
        trimmed += 1;
    }
    while trimmed > 0 {
        let last = segments.last_mut()?;
        let cells = term
            .bounds_to_string(*last.start(), *last.end())
            .chars()
            .count();
        if trimmed < cells {
            *last = *last.start()..=last.end().sub(term, Boundary::Grid, trimmed);
            break;
        }
        segments.pop();
        trimmed -= cells;
    }
    segments
        .iter()
        .any(|run| run.contains(&point))
        .then_some((segments, url))
}

/// Runs of characters a URL may contain on the line (soft wraps joined) through `point`.
fn url_runs(term: &Term<Listener>, point: Point) -> Vec<Match> {
    let start = term.line_search_left(point);
    let end = term.line_search_right(point);
    URL_CHARS.with_borrow_mut(|regex| match regex.as_mut() {
        Some(regex) => RegexIter::new(start, end, Direction::Right, term, regex).collect(),
        None => Vec::new(),
    })
}

/// Whether a program broke a URL off between two lines, given as their URL runs: the
/// upper line ends in a run that reaches at least as close to the right edge as the
/// lower line is indented, and the lower line starts with a run at that indentation.
fn continues(term: &Term<Listener>, upper: &[Match], lower: &[Match]) -> bool {
    let (Some(last), Some(first)) = (upper.last(), lower.first()) else {
        return false;
    };
    let (end, start) = (*last.end(), *first.start());
    let indent = start.column.0;
    // Whether the cells `from..to` of a row are blank.
    let blank = |line: Line, from: usize, to: usize| {
        from >= to
            || term
                .bounds_to_string(
                    Point::new(line, Column(from)),
                    Point::new(line, Column(to - 1)),
                )
                .trim()
                .is_empty()
    };
    end.line == term.line_search_right(end).line
        && end.column.0 + indent + 1 >= term.columns()
        && blank(end.line, end.column.0 + 1, term.columns())
        && start.line == term.line_search_left(start).line
        && blank(start.line, 0, indent)
}

/// Converts a mouse position in cell units to a grid point and the half of
/// the cell it falls in. Positions outside the screen are clamped to it.
fn selection_point(term: &Term<Listener>, col: f32, row: f32) -> (Point, Side) {
    let last_col = term.columns() - 1;
    let column = (col.max(0.0) as usize).min(last_col);
    let line = (row.max(0.0) as usize).min(term.screen_lines() - 1);
    let side = if col.fract() >= 0.5 || col as usize > last_col {
        Side::Right
    } else {
        Side::Left
    };
    let point = Point::new(line, Column(column));
    (viewport_to_point(term.grid().display_offset(), point), side)
}

/// Drops trailing blanks that draw nothing.
fn trim_trailing_blanks(runs: &mut Vec<Run>) {
    let Some(last) = runs.last_mut() else {
        return;
    };
    let style = last.style;
    let draws_background = style.bg != AnsiColor::Named(NamedColor::Background)
        || style
            .flags
            .intersects(Flags::INVERSE | Flags::ALL_UNDERLINES);
    if draws_background {
        return;
    }
    let trimmed = last.text.trim_end_matches(' ').len();
    last.width -= last.text.len() - trimmed;
    last.text.truncate(trimmed);
    if last.text.is_empty() {
        runs.pop();
    }
}

fn to_ui(run: &Run, colors: &Colors) -> TermRun {
    let style = run.style;
    let mut fg = rgb_color(resolve(style.fg, colors));
    let mut bg = rgb_color(resolve(style.bg, colors));
    if style.flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    } else if style.bg == AnsiColor::Named(NamedColor::Background) {
        bg = Color::from_argb_u8(0, 0, 0, 0);
    }
    if style.flags.contains(Flags::DIM) {
        fg = fg.with_alpha(0.6);
    }
    TermRun {
        col: run.col as i32,
        width: run.width as i32,
        text: run.text.as_str().into(),
        fg,
        bg,
        bold: style.flags.contains(Flags::BOLD),
        italic: style.flags.contains(Flags::ITALIC),
        underline: style.flags.intersects(Flags::ALL_UNDERLINES),
    }
}

fn rgb_color((r, g, b): (u8, u8, u8)) -> Color {
    Color::from_rgb_u8(r, g, b)
}

/// Resolves a cell color. Colors set by programs (OSC 4/10/11) win over the palette.
fn resolve(color: AnsiColor, colors: &Colors) -> (u8, u8, u8) {
    let index = match color {
        AnsiColor::Spec(rgb) => return (rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(index) => index as usize,
        AnsiColor::Named(name) => name as usize,
    };
    colors[index].map_or_else(|| palette(index), |rgb| (rgb.r, rgb.g, rgb.b))
}

/// The default color for a palette index (0-255) or a named color.
fn palette(index: usize) -> (u8, u8, u8) {
    let dim = NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize;
    match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            let i = index - 16;
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let v = (8 + (index - 232) * 10) as u8;
            (v, v, v)
        }
        _ if index == NamedColor::Background as usize => WELL_BG,
        _ if dim.contains(&index) => {
            let (r, g, b) = ANSI[index - NamedColor::DimBlack as usize];
            (r / 3 * 2, g / 3 * 2, b / 3 * 2)
        }
        _ => DEFAULT_FG,
    }
}

/// Translates a key event into the bytes a terminal sends for it.
fn key_bytes(text: &str, ctrl: bool, shift: bool, app_cursor: bool) -> Option<Vec<u8>> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return Some(text.as_bytes().to_vec());
    }

    if let Some((_, sequence)) = KEY_SEQUENCES.iter().find(|(key, _)| char::from(*key) == c) {
        return Some(sequence.as_bytes().to_vec());
    }
    if c == char::from(Key::Tab) {
        return Some(if shift {
            b"\x1b[Z".to_vec()
        } else {
            b"\t".to_vec()
        });
    }
    let arrow = [
        (Key::UpArrow, b'A'),
        (Key::DownArrow, b'B'),
        (Key::RightArrow, b'C'),
        (Key::LeftArrow, b'D'),
    ];
    if let Some((_, letter)) = arrow.iter().find(|(key, _)| char::from(*key) == c) {
        let prefix = if app_cursor { b'O' } else { b'[' };
        return Some(vec![0x1b, prefix, *letter]);
    }

    if ctrl {
        let byte = match c.to_ascii_lowercase() {
            letter @ 'a'..='z' => letter as u8 & 0x1f,
            ' ' | '@' | '2' => 0x00,
            '[' | '3' => 0x1b,
            '\\' | '4' => 0x1c,
            ']' | '5' => 0x1d,
            '^' | '6' => 0x1e,
            '_' | '-' | '7' => 0x1f,
            '?' | '8' => 0x7f,
            // Some platforms already deliver the control character.
            c if (c as u32) < 0x20 => c as u8,
            _ => return None,
        };
        return Some(vec![byte]);
    }

    // Modifier keys and other non-text keys arrive as control or
    // private-use characters; they produce no input.
    if (c as u32) < 0x20 || ('\u{E000}'..='\u{F8FF}').contains(&c) {
        return None;
    }
    let mut buffer = [0u8; 4];
    Some(c.encode_utf8(&mut buffer).as_bytes().to_vec())
}
