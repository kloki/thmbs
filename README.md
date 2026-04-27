# thmbs

`ls(1)`-style listing supplemented with inline image thumbnails and dimensions,
rendered via the iTerm2 inline-images escape sequence.

## Origin

This is an automated LLM rewrite (Perl → Rust) of iTerm2's `imgls` utility:
<https://iterm2.com/utilities/imgls>.

The original is a Perl script bundled with iTerm2's shell integration. This
project ports the behaviour to a single Rust binary while preserving the
command-line interface.

## Usage

```
thmbs --help
```

Requires a terminal that understands the iTerm2 inline-images protocol
(iTerm2, WezTerm, etc.).
