use std::io::{Read, stdin};

use anyhow::Result;
use crossterm::event::{Event, read};

fn main() -> Result<()> {
    crossterm::terminal::enable_raw_mode()?;

    let mut stdin = stdin();
    let mut buf = [0; 5];

    for _ in 0..20 {
        let bytes_read = stdin.read(&mut buf);

        dbg!(buf);
    }

    for _ in 0..20 {
        if let Event::Key(event) = read()? {
            println!("{event:?}");
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}
