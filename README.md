# Typyr

Shows your key presses on the command line (look at the bottom row of the video)

[![asciicast](https://asciinema.org/a/XoDe8hDYMNVcHvrkyzK4djXSo.svg)](https://asciinema.org/a/XoDe8hDYMNVcHvrkyzK4djXSo)

## Usage

```bash
typyr
```

## Is it blazingly fast...?

Even though it's written in Rust, probably not.

## How it works

ratatui + tui_term + portable_pty

## Limitations

It probably lacks more things,
but currently it works (well enough) for my own use case.

- [ ] Lack of scroll support
  - Since application opens in alternate screen,
    the app itself needs to handle scroll (which I'm too lazy to do, for now)
- [ ] Lack of mouse support
  - Too much work, just don't use the mouse (I use vim btw)
- [ ] Doesn't support all escape sequences
  - This is a limitation of the [`vt100`](https://docs.rs/vt100/0.16.2/vt100/) crate,
    however, it is also non-trivial to parse all escape sequences
  - If you use `fzf` for `ctrl r` shell history, then it might lag/does not open
  - It takes a while to close Vim because of this issue
- [ ] Not tested on all platforms
  - I only tested it on Linux (Fedora FYI)
