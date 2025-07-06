use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::thread;

use crossterm::cursor::{MoveTo, MoveUp, RestorePosition, SavePosition};
use crossterm::execute;
use crossterm::style::{Print, ResetColor};
use crossterm::terminal::{
    Clear, ClearType, ScrollDown, ScrollUp, SetSize, WindowSize, window_size,
};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

use anyhow::Result;

#[allow(clippy::type_complexity)]
fn spawn_shell() -> Result<(
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
    let cmd = CommandBuilder::new("tmux");
    let child = pair.slave.spawn_command(cmd)?;

    // Read and parse output from the pty with reader
    let reader = pair.master.try_clone_reader()?;

    // Send data to the pty by writing to the master
    let writer = pair.master.take_writer()?;

    Ok((child, reader, writer))
}

fn main() -> Result<()> {
    crossterm::terminal::enable_raw_mode()?;

    let (mut child, mut reader, mut writer) = spawn_shell()?;

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

        let stdin = io::stdin();
        let mut handle = stdin.lock();
        let mut buf = [0; 1024];
        while let Ok(n) = handle.read(&mut buf) {
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).unwrap();
            writer.flush().unwrap();

            write!(file, "{:?}", &buf[..n]).unwrap();
            file.flush().unwrap();
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

            // execute!(
            //     stdout,
            //     // SavePosition,
            //     // MoveTo(0, rows - 1), // Move to bottom-left
            //     // ResetColor,
            //     // Clear(ClearType::CurrentLine),
            //     // Print("Y".repeat(columns as usize)),
            //     // RestorePosition,
            // )
            // .unwrap();
        }
    });

    let status = child.wait()?;
    crossterm::terminal::disable_raw_mode()?;

    dbg!(status);

    Ok(())
}
