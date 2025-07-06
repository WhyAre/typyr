use crossterm::cursor::MoveTo;
use crossterm::execute;
use crossterm::terminal::{Clear, ClearType};
use itertools::Itertools;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::process::Command;
use std::thread;

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
        rows: window_size.rows,
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
    let cmd = CommandBuilder::new(cmd);
    let child = pair.slave.spawn_command(cmd)?;

    // Read and parse output from the pty with reader
    let reader = pair.master.try_clone_reader()?;

    // Send data to the pty by writing to the master
    let writer = pair.master.take_writer()?;

    Ok((child, reader, writer))
}

fn main() -> Result<()> {
    let cmd = std::env::args().nth(1).expect("missing argument");

    crossterm::terminal::enable_raw_mode()?;

    let (mut child, mut reader, mut writer) = spawn_shell(&cmd)?;

    // let WindowSize { rows, columns, .. } = window_size()?;

    let mut stdout = io::stdout();
    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;

    let _ = thread::spawn(move || {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open("/tmp/file.txt")
            .unwrap();
        let mut tmux_refresh = Command::new("tmux");

        let stdin = io::stdin();
        let mut handle = stdin.lock();
        let mut buf = [0; 1024];
        while let Ok(n) = handle.read(&mut buf) {
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).unwrap();
            writer.flush().unwrap();

            let out_str: String = buf[..n].iter().map(pretty_display).collect();
            write!(file, "{out_str}").unwrap();
            file.flush().unwrap();

            tmux_refresh
                .arg("refresh-client")
                .arg("-S")
                .output()
                .unwrap();
        }
    });

    // Read output from PTY and print it to screen
    let _ = thread::spawn(move || {
        let mut stdout = io::stdout().lock();
        let mut buf = [0; 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                continue;
            }

            stdout.write_all(&buf[..n]).unwrap();
            stdout.flush().unwrap();
        }
    });

    let status = child.wait()?;
    crossterm::terminal::disable_raw_mode()?;

    dbg!(status);

    Ok(())
}
