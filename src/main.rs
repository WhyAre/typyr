use std::{
    collections::VecDeque,
    fmt::Display,
    fs::OpenOptions,
    io::{Read, Write, stdout},
    sync::{
        Arc, RwLock,
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

use itertools::Itertools;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use ratatui::{
    Frame,
    crossterm::{
        cursor::Show,
        terminal::{LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, window_size},
    },
    layout::Alignment,
    style::{Modifier, Style},
    widgets::Paragraph,
};
use ratatui::{
    crossterm::{execute, terminal::EnterAlternateScreen},
    termwiz::{
        self,
        input::{InputEvent, KeyEvent, Modifiers},
        terminal::Terminal,
    },
};
use ratatui::{prelude::CrosstermBackend, termwiz::input::KeyCode};
use shell_words::split;
use tracing::{debug, info, level_filters::LevelFilter};
use tui_term::widget::PseudoTerminal;
use vt100::Screen;

#[derive(Debug)]
struct History {
    history: VecDeque<KeyEvent>,
    cur_width: usize,
    max_width: usize,
}

impl Display for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fmt = self.history.iter().map(Self::prettify_keycode).join("");
        write!(f, "{fmt}")
    }
}

impl History {
    fn new(limit: usize) -> Self {
        Self {
            history: VecDeque::new(),
            cur_width: 0,
            max_width: limit,
        }
    }

    fn push(&mut self, item: KeyEvent) {
        self.cur_width += Self::prettify_keycode(&item).chars().count();

        self.history.push_back(item);

        while self.cur_width > self.max_width {
            let removed_item = self.history.pop_front().expect("Deque is empty");
            self.cur_width -= Self::prettify_keycode(&removed_item).chars().count();
        }

        debug_assert!(self.cur_width <= self.max_width);
    }

    fn clear(&mut self) {
        self.history.clear();
        self.cur_width = 0;
    }

    fn prettify_keycode(e: &KeyEvent) -> String {
        let KeyEvent { key, modifiers } = e;

        let mut flags = Vec::new();

        let key = match key {
            KeyCode::Char(c) => match c {
                ' ' => "\u{2423}".to_string(),
                '\0' => {
                    flags.push("C");
                    "\u{2423}".to_string()
                }
                _ => c.to_string(),
            },
            KeyCode::Backspace => "BS".to_string(),
            KeyCode::Enter => "CR".to_string(),
            KeyCode::Escape => "Esc".to_string(),
            KeyCode::LeftArrow => "Left".to_string(),
            KeyCode::RightArrow => "Right".to_string(),
            KeyCode::UpArrow => "Up".to_string(),
            KeyCode::DownArrow => "Down".to_string(),
            _ => format!("{key:?}"),
        };

        if modifiers.contains(Modifiers::CTRL) {
            flags.push("C");
        }
        if modifiers.contains(Modifiers::ALT) {
            flags.push("M");
        }

        if flags.is_empty() {
            if key.chars().count() == 1 {
                key
            } else {
                format!("<{key}>")
            }
        } else {
            format!("<{}-{}>", flags.join("-"), key)
        }
    }
}

struct Pty {
    master: Box<dyn MasterPty>,
    process: Box<dyn Child + Send + 'static>,
    size: PtySize,
}

fn spawn_command(cmd: &str, args: &[&str]) -> anyhow::Result<Pty> {
    let pty_system = portable_pty::native_pty_system();

    let cwd = std::env::current_dir()?;
    let mut cmd = CommandBuilder::new(cmd);
    cmd.cwd(cwd);
    cmd.args(args);

    let window_size = window_size().expect("Cannot get window size");

    info!("{window_size:?}");

    let size = PtySize {
        rows: window_size.rows - 1,
        cols: window_size.columns,
        pixel_width: window_size.width,
        pixel_height: window_size.height,
    };

    let pair = pty_system.openpty(size)?;

    // Wait for the child to complete
    let child = pair.slave.spawn_command(cmd)?;

    Ok(Pty {
        master: pair.master,
        process: child,
        size,
    })
}

#[derive(Debug)]
enum UIEvent {
    Update,
}
use clap::{Parser, ValueEnum};

#[derive(clap::Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Set the log level
    #[arg(short = 'l', long = "log-level", value_enum, default_value = "off")]
    log_level: LogLevel,

    /// Command to execute
    #[arg()]
    cmd: Option<String>,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum LogLevel {
    Off,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<LogLevel> for LevelFilter {
    fn from(value: LogLevel) -> Self {
        match value {
            LogLevel::Off => Self::OFF,
            LogLevel::Debug => Self::DEBUG,
            LogLevel::Info => Self::INFO,
            LogLevel::Warn => Self::WARN,
            LogLevel::Error => Self::ERROR,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();

    // Read the argument
    let cmd = args
        .cmd
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or("bash".to_string());

    // Set up logging
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("typyr.log")
        .expect("Cannot open log file");

    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(file)
        .with_max_level(args.log_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Unable to set global subscriber");

    let split = split(&cmd)?;
    let Pty {
        master,
        mut process,
        size,
    } = spawn_command(
        &split[0],
        &split.iter().skip(1).map(|c| c.as_ref()).collect::<Vec<_>>(),
    )?;
    let mut reader = master.try_clone_reader().expect("Cannot get reader");

    execute!(stdout(), EnterAlternateScreen)?;
    enable_raw_mode()?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;

    let (tx, rx) = mpsc::channel();

    let parser = Arc::new(RwLock::new(vt100::Parser::new(size.rows, size.cols, 0)));
    let history = Arc::new(RwLock::new(History::new(size.cols as usize - 2))); // -2 for 1 row of margins on each side

    // Read from stdin, and forward into the application
    {
        let tx = tx.clone();
        let history = history.clone();
        let parser = parser.clone();
        thread::spawn(|| run(master, tx, parser, history));
    }

    // Read from stdout, and forward into the screen
    {
        let parser = parser.clone();
        let tx = tx.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];

            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    continue;
                }

                parser
                    .write()
                    .expect("Cannot write to parser")
                    .process(&buf[..n]);
                tx.send(UIEvent::Update).expect("Fail to send UIEvent");
            }
        });
    }

    //Updates the UI
    thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            debug_assert!(matches!(event, UIEvent::Update));

            let parser_read = parser.read().expect("Cannot read parser");
            let screen = parser_read.screen();
            let history = history.read().expect("Cannot read history");

            terminal
                .draw(|f| ui(f, screen, history.to_string()))
                .unwrap();
        }
    });

    process.wait().unwrap();

    disable_raw_mode()?;
    // I have no idea why I need "Show" here...
    // but the cursor doesn't show if I don't
    // "Show"
    execute!(stdout(), LeaveAlternateScreen, Show)?;

    Ok(())
}

fn run(
    master: Box<dyn MasterPty>,
    tx: Sender<UIEvent>,
    parser: Arc<RwLock<vt100::Parser>>,
    history: Arc<RwLock<History>>,
) {
    let mut t1 =
        termwiz::terminal::new_terminal(termwiz::caps::Capabilities::new_from_env().unwrap())
            .unwrap();

    let mut sender = master.take_writer().expect("Cannot get writer");

    // Busy waiting
    loop {
        let Some(event) = t1
            .poll_input(Some(Duration::from_millis(10)))
            .expect("Fail to get InputEvent")
        else {
            continue;
        };

        info!("{event:?}");

        // Process event
        match event {
            InputEvent::Key(event) => {
                let mut history = history
                    .write()
                    .expect("Cannot get writer to history object");

                if event
                    == (KeyEvent {
                        key: KeyCode::Char('\u{1c}'),
                        modifiers: Modifiers::NONE,
                    })
                {
                    history.clear();
                    tx.send(UIEvent::Update).expect("Fail to send UIEvent");
                    continue;
                }

                history.push(event.clone());
                tx.send(UIEvent::Update).expect("Fail to send UIEvent");

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

                sender
                    .write_all(key_encoding.as_bytes())
                    .expect("Fail to write to pty stdin");
                sender.flush().unwrap();
            }
            InputEvent::Resized { cols, rows } => {
                let (rows, cols) = ((rows - 1) as u16, cols as u16); // Always remember size for my bottom bar
                let cursize = master.get_size().expect("Cannot get size");

                debug!("Resized to {rows:?} {cols:?}");

                master
                    .resize(PtySize {
                        rows,
                        cols,
                        ..cursize
                    })
                    .expect("Cannot resize");
                parser
                    .write()
                    .expect("Cannot get parser")
                    .set_size(rows, cols);
            }
            _ => {}
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
        .alignment(Alignment::Center);
    f.render_widget(keystrokes, chunks[1]);
}
