use std::{
    collections::VecDeque,
    fmt::Display,
    fs::File,
    io::{self, Read, Write, stdout},
    sync::{Arc, RwLock},
    thread::{self, sleep},
    time::Duration,
};

use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor::MoveTo, terminal::WindowSize};
use crossterm::{execute, terminal::window_size};
use futures::FutureExt;
use itertools::Itertools;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use ratatui::termwiz::input::KeyCode;
use ratatui::termwiz::{
    self,
    input::{InputEvent, KeyEvent, Modifiers},
    terminal::Terminal,
};
use ratatui::{
    Frame,
    layout::Alignment,
    prelude::TermwizBackend,
    style::{Modifier, Style},
    widgets::Paragraph,
};
use shell_words::split;
use std::fs::OpenOptions;
use std::process::Command;
use tokio::{sync::Notify, task};
use tui_term::vt100::{Callbacks, Parser, Screen};
use tui_term::widget::PseudoTerminal;

use anyhow::Result;

/// Just write enough code until this is correct... please tell me if there's a library for this
fn pretty_display(code: &u8) -> String {
    match code {
        1..=26 => format!("<C-{}>", (b'A' + code - 1) as char),
        27 => "<Esc>".to_string(),
        32..=126 => (*code as char).to_string(), // printable ASCII
        127 => "<BS>".to_string(),
        _ => format!("<0x{code:02X}>"),
    }
}
fn parse_command(cmd: &str) -> CommandBuilder {
    let mut parts = split(cmd).expect("Invalid shell command");
    let prog = parts.remove(0);
    let pwd = std::env::var("PWD").unwrap();
    let mut cmd = CommandBuilder::new(prog);
    cmd.args(parts);
    cmd.cwd(pwd);
    for (key, value) in std::env::vars() {
        cmd.env(&key, &value);
    }
    cmd
}
#[allow(clippy::type_complexity)]
fn spawn_shell(
    cmd: &str,
) -> Result<(
    Box<dyn Child + Send + Sync + 'static>,
    Box<dyn Read + Send + 'static>,
    Box<dyn Write + Send + 'static>,
)> {
    let pty_system = native_pty_system();

    // Create a new pty
    let window_size = crossterm::terminal::window_size()?;
    let pair = pty_system.openpty(PtySize {
        rows: window_size.rows - 1,
        cols: window_size.columns,
        // Not all systems support pixel_width, pixel_height,
        // but it is good practice to set it to something
        // that matches the size of the selected font.  That
        // is more complex than can be shown here in this
        // brief example though!
        pixel_width: window_size.width,
        pixel_height: window_size.height,
    })?;

    // Spawn a shell into the pty
    let cmd = parse_command(cmd);
    let child = pair.slave.spawn_command(cmd)?;

    // Read and parse output from the pty with reader
    let reader = pair.master.try_clone_reader()?;

    // Send data to the pty by writing to the master
    let writer = pair.master.take_writer()?;

    Ok((child, reader, writer))
}

struct Testing {
    fd: File,
}

impl Testing {
    fn new() -> Self {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open("cb.log")
            .unwrap();

        Self { fd: file }
    }
}

impl Callbacks for Testing {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {}

    fn visual_bell(&mut self, _: &mut vt100::Screen) {}

    fn resize(&mut self, _: &mut vt100::Screen, _request: (u16, u16)) {}

    fn set_window_icon_name(&mut self, _: &mut vt100::Screen, _icon_name: &[u8]) {}

    fn set_window_title(&mut self, _: &mut vt100::Screen, _title: &[u8]) {}

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], _data: &[u8]) {}

    fn paste_from_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8]) {}

    fn unhandled_char(&mut self, _: &mut vt100::Screen, _c: char) {
        writeln!(self.fd, "unhandled_char: {_c}");
        self.fd.flush().unwrap();
    }

    fn unhandled_control(&mut self, _: &mut vt100::Screen, _b: u8) {
        writeln!(self.fd, "unhandled_control: {_b}");
        self.fd.flush().unwrap();
    }

    fn unhandled_escape(
        &mut self,
        _: &mut vt100::Screen,
        _i1: Option<u8>,
        _i2: Option<u8>,
        _b: u8,
    ) {
        writeln!(self.fd, "unhandled_escape: {_i1:?}");
        writeln!(self.fd, "unhandled_escape: {_i2:?}");
        writeln!(self.fd, "unhandled_escape: {_b}");
        self.fd.flush().unwrap();
    }

    fn unhandled_csi(
        &mut self,
        _: &mut vt100::Screen,
        _i1: Option<u8>,
        _i2: Option<u8>,
        _params: &[&[u16]],
        _c: char,
    ) {
        fn to_csi(i1: Option<u8>, i2: Option<u8>, params: &[&[u16]], final_char: char) -> String {
            let mut out = "\x1B[".to_string();
            if !params.is_empty() {
                let joined = params
                    .iter()
                    .map(|g| g.iter().map(u16::to_string).collect::<Vec<_>>().join(";"))
                    .collect::<Vec<_>>()
                    .join(";");
                out.push_str(&joined);
            }
            if let Some(i) = i1 {
                out.push(i as char);
            }
            if let Some(i) = i2 {
                out.push(i as char);
            }
            out.push(final_char);
            out
        }

        // writeln!(self.fd, "Unhandled CSI");
        // writeln!(self.fd, "{_i1:?}");
        // writeln!(self.fd, "{_i2:?}");
        // writeln!(self.fd, "{_params:?}");
        // writeln!(self.fd, "{_c}");
        // self.fd.flush().unwrap();

        let out = to_csi(_i1, _i2, _params, _c);
        writeln!(stdout(), "{out}");
        stdout().flush().unwrap();
        writeln!(self.fd, "Out: {out}");
        self.fd.flush().unwrap();
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, _params: &[&[u8]]) {
        fn reconstruct_osc_string(params: &[&[u8]]) -> String {
            let mut s = String::from("\x1B]");
            let parts: Vec<String> = params
                .iter()
                .map(|group| String::from_utf8_lossy(group).into_owned())
                .collect();
            s.push_str(&parts.join(";"));
            s.push('\x07'); // use BEL to terminate
            s
        }
        // writeln!(self.fd, "unhandled_osc: {_params:?}");
        // self.fd.flush().unwrap();
        let out = reconstruct_osc_string(_params);
        writeln!(stdout(), "{out}");
        stdout().flush().unwrap();
    }
}

fn main() -> Result<()> {
    let cmd = std::env::args().nth(1).expect("missing argument");

    let backend = TermwizBackend::new().unwrap();
    let mut terminal = ratatui::Terminal::new(backend)?;
    //
    // crossterm::terminal::enable_raw_mode()?;

    let (mut child, mut reader, mut writer) = spawn_shell(&cmd)?;

    let size = window_size()?;

    // let mut stdout = io::stdout();
    // execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;

    let _ = thread::spawn(move || {
        run(writer).unwrap();
    });

    // Read output from PTY and print it to screen
    let _ = thread::spawn(move || {
        let mut stdout = io::stdout().lock();
        let mut buf = [0; 8192];
        let mut wr = OpenOptions::new()
            .append(true)
            .create(true)
            .open("what.log")
            .unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open("parser.log")
            .unwrap();
        let mut parser = vt100::Parser::<Testing>::new_with_callbacks(
            size.rows,
            size.columns,
            0,
            Testing::new(),
        );
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                continue;
            }

            // parser.process(&buf[..n]);
            // writeln!(file, "{n}").unwrap();
            // file.flush().unwrap();
            // parser.flush();
            stdout.write_all(&buf[..n]).unwrap();
            stdout.flush().unwrap();
            writeln!(wr, "{:?}", String::from_utf8(buf[..n].to_vec())).unwrap();
            wr.flush().unwrap();
            // terminal
            //     .draw(|f| ui(f, parser.screen(), "Hello".to_string()))
            //     .unwrap();
            // stdout.flush().unwrap();
        }
    });

    let status = child.wait()?;
    // crossterm::terminal::disable_raw_mode()?;

    dbg!(status);

    Ok(())
}

fn run(mut sender: Box<dyn Write + Send + 'static>) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open("parser.log")
        .unwrap();

    let mut t1 =
        termwiz::terminal::new_terminal(termwiz::caps::Capabilities::new_from_env().unwrap())
            .unwrap();

    // let mut history = History::new(40);

    loop {
        if let Some(event) = t1.poll_input(None).unwrap() {
            // Process event
            if let InputEvent::Key(event) = event {
                if event
                    == (KeyEvent {
                        key: KeyCode::Char('\\'),
                        modifiers: Modifiers::CTRL,
                    })
                {
                    // history.clear();
                    continue;
                }

                // history.push(event.clone());

                let _ = writeln!(file, "KeyEvent: {event:?}");
                let _ = writeln!(file, "{:?}", event.key);
                let key_encoding = event
                    .key
                    .encode(
                        event.modifiers,
                        termwiz::input::KeyCodeEncodeModes {
                            encoding: termwiz::input::KeyboardEncoding::Xterm,
                            application_cursor_keys: false,
                            newline_mode: false,
                            modify_other_keys: None,
                        },
                        true,
                    )
                    .unwrap();
                sender.write_all(key_encoding.as_bytes())?;
                sender.flush().unwrap();
            }

            let _ = file.flush();
        }
    }
}

fn ui(f: &mut Frame, screen: &Screen, history: String) {
    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints(
            [
                ratatui::layout::Constraint::Percentage(100),
                ratatui::layout::Constraint::Min(1),
            ]
            .as_ref(),
        )
        .split(f.area());
    let pseudo_term = PseudoTerminal::new(screen);
    f.render_widget(pseudo_term, chunks[0]);

    let keystrokes = Paragraph::new(history)
        .style(Style::default().add_modifier(Modifier::REVERSED))
        .alignment(Alignment::Right);
    f.render_widget(keystrokes, chunks[1]);
}
