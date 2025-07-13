# Typyr

Shows your key presses on the command line

## Is it blazingly fast...?

Even though it's written in Rust, probably not.

## Limitations

- [ ] Lack of scroll support
     - Since application opens in alternate screen,
       the app itself needs to handle scroll (which I'm too lazy to do, for now)
- [ ] Doesn't support all escape sequences
     - If you use `fzf` for `ctrl r` shell history, then it might lag/does not open
     - It takes a while to close Vim because of this issue
