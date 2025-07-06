use std::io::{self, Read, Write, stdin, stdout};
use std::thread;

use crossterm::event::{Event, read};
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
        rows: window_size.rows - 10,
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
    let cmd = CommandBuilder::new("bash");
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

    let input_thread = thread::spawn(move || {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        let mut buf = [0; 1024];
        while let Ok(n) = handle.read(&mut buf) {
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).unwrap();
            writer.flush().unwrap();
        }
    });

    // Read output from PTY and print it to screen
    let output_thread = thread::spawn(move || {
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

