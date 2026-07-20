# cutback

Version control for Kdenlive projects. It watches your project, commits every
save to a local git repository, and tells you what changed in sentences instead
of XML.

```
$ cutback watch
recorded the current state of holiday.kdenlive
watching /home/you/videos/holiday/holiday.kdenlive
stop with Ctrl-C

  (you save in Kdenlive)

Added intro-title to the project bin and 3 more changes
  added intro-title to the project bin
  added intro-title to V2 at 0:14, 0:05 long

$ cutback log
12c916c      just now  Changed gain 0.25 to 0.90 on Wild Side - ALI.mp3, A1
1305721     3 min ago  Added intro-title to the project bin and 3 more changes
445258d    22 min ago  Started watching this project

$ cutback diff
changed gain 0.25 to 0.90 on Wild Side - ALI.mp3, A1
```

**Kdenlive only, Linux only, right now.** Premiere support is the planned next
target, but it is not built and this repository contains none of it.

## Install

Needs a Rust toolchain from [rustup.rs](https://rustup.rs). Then:

```sh
git clone https://github.com/xevrion/cutback
cd cutback
./install.sh
```

That builds from source and installs `cutback` and its man page into
`~/.local`. Use `--prefix /usr/local` for a system wide install, and
`./install.sh --uninstall` to remove it.

## Use

Start the watcher in your project folder and leave it running while you edit:

```sh
cd ~/videos/my-project
cutback watch
```

Then save in Kdenlive as usual. Every save that actually changes something
becomes a commit.

The other five commands:

```sh
cutback log                   # what has happened, newest first
cutback diff                  # what the last save changed
cutback diff 8126417 5ee6275  # what changed between two points
cutback restore 5ee6275       # put the project back to that state
cutback branch short-version  # start an alternate cut
cutback checkout short-version
```

Close the project in Kdenlive before `restore` or `checkout`. Kdenlive holds
the document in memory and will write over the restored file when it next
saves.

Run `man cutback` for the full documentation.

## How it works

The history lives in a `.cutback` directory next to your project. It is an
ordinary git repository, so nothing is locked away in a format only this tool
can read. Your own `.git` directory, if the folder has one, is untouched.

Kdenlive project files reference footage by path rather than embedding it, so
the history stays small no matter how much footage you have. Only the project
file is tracked.

A restore writes back exactly the bytes Kdenlive wrote. cutback reads the XML
to describe changes but never rewrites it, which matters because a project
using track wide effects is not valid XML to generic tools.

## What is not here

No merge. This is a solo tool: one person, one machine, no team features and no
cloud. No graphical interface.

Projects must be document version 1.1, which Kdenlive 23.04 and later write.
Older projects are refused with a message rather than parsed approximately.
Opening one in Kdenlive and saving it upgrades the format.

## Building and testing

```sh
cargo build --release
cargo test
```

The tests run against real `.kdenlive` files in `tests/data`, written by
Kdenlive itself rather than authored by hand. Three of them are consecutive
saves of one project, which is what the differ is checked against. Two of those
saves differ only in how Kdenlive ordered the XML, with nothing actually
edited, and the differ is expected to report no changes for that pair.

## Credit

Built against the Kdenlive project's own
[file format documentation](https://github.com/KDE/kdenlive/blob/master/dev-docs/fileformat.md),
which is the primary reference for everything in `xml_parser`. Two details in
this tool came from reading Kdenlive's source rather than the docs: current
versions write media clips as `<chain>` rather than `<producer>`, and markers
are stored as a JSON array with frame positions rather than the older
`position:comment` form.

Uses [MLT](https://www.mltframework.org/), the framework Kdenlive's file format
is based on, by way of that format.
