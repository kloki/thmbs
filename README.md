# thmbs

`ls(1)`-style listing supplemented with inline image thumbnails and dimensions, rendered via the iTerm2 inline-images escape sequence.

## How it works

`thmbs` walks the given paths like `ls`, then for each image file emits an iTerm2 inline-image escape sequence so the terminal renders a small thumbnail next to the filename. Image dimensions (WxH) can be shown alongside. Requires a terminal that understands the iTerm2 inline-images protocol (iTerm2, WezTerm, etc.).

## Origin

This is an automated LLM rewrite (Perl → Rust) of iTerm2's `imgls` utility: <https://iterm2.com/utilities/imgls>. The original is a Perl script bundled with iTerm2's shell integration. This project ports the behaviour to a single Rust binary while preserving the command-line interface.

## Install

### Binaries

Check [Releases](https://github.com/kloki/thmbs/releases) for binaries and installers.

```bash
# List the current directory with thumbnails
thmbs

# List a specific directory in long format with dimensions
thmbs -l --dimensions ~/Pictures

# Larger thumbnails
thmbs --width 6 --height 3

# See all options
thmbs --help
```
