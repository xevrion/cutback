cutback
=======

cutback is version control for Kdenlive projects. It watches a project
folder, commits every save to a local git repository without being asked,
and describes what changed between any two points in plain English rather
than as a diff of MLT XML.

Video editors do not really have version control. People save over their own
work, lose an afternoon to a crash, and have no way to see what changed
between two versions of a project except opening both and squinting. cutback
is a narrow fix for that, for Kdenlive, on Linux.

    $ cutback watch
    recorded the current state of holiday.kdenlive
    watching /home/you/videos/holiday/holiday.kdenlive
    stop with Ctrl-C

    Added intro-title to the project bin and 3 more changes
      added intro-title to the project bin
      added intro-title to V2 at 0:14, 0:05 long

    $ cutback log
    12c916c      just now  Changed gain 0.25 to 0.90 on Wild Side - ALI.mp3, A1
    1305721     3 min ago  Added intro-title to the project bin and 3 more changes
    445258d    22 min ago  Started watching this project

    $ cutback diff
    changed gain 0.25 to 0.90 on Wild Side - ALI.mp3, A1

Kdenlive only, Linux only, right now. Premiere is the planned next target
but none of it is built, and this repository contains no support for any
other editor.

Installing
----------

cutback is built from source and needs a Rust toolchain, which you can get
from https://rustup.rs.

    git clone https://github.com/xevrion/cutback
    cd cutback
    ./install.sh

That installs the binary, the man page and shell completions under
`~/.local`. Pass `--prefix /usr/local` for a system wide install, and
`./install.sh --uninstall` to remove it again. Uninstalling leaves your
project histories alone.

Using it
--------

Start the watcher in your project folder and leave it running while you
edit. Every save that changes something becomes a commit.

    cd ~/videos/my-project
    cutback watch

The rest of the commands read that history.

    cutback log                   what has happened, newest first
    cutback diff                  what the last save changed
    cutback diff 8126417 5ee6275  what changed between two points
    cutback restore 5ee6275       put the project back to that state
    cutback branch short-version  start an alternate cut
    cutback checkout short-version

Close the project in Kdenlive before restoring or switching branches.
Kdenlive holds the document in memory and will write it back over the
restored file the next time it saves.

There is a man page. Run `man cutback`.

How it works
------------

The history lives in a `.cutback` directory beside your project. It is an
ordinary git repository, so nothing is trapped in a format only this tool
can read, and your own `.git` directory is never touched if the folder has
one.

Only the project file is tracked. Kdenlive references footage by path
instead of embedding it, so the history stays small no matter how much
footage the project uses.

A restore writes back exactly the bytes Kdenlive wrote. cutback reads the
XML to work out what changed, but it never rewrites it. That matters because
a project using track wide effects is not valid XML to generic tools, a
gotcha documented by the Kdenlive developers themselves.

Saves are detected by watching for the rename that completes them rather
than by waiting a fixed interval. Kdenlive saves through QSaveFile, which
writes a temporary file and renames it into place, so a reader sees either
the whole old file or the whole new one and never a half written project.

What is not here
----------------

There is no merge. This is a tool for one person on one machine, with no
team features, no cloud and no graphical interface.

Projects must be document version 1.1, which Kdenlive 23.04 and later write.
Older projects are refused with a message rather than parsed approximately,
because a wrong diff is worse than no diff. Opening an old project in
Kdenlive and saving it upgrades the format.

Hacking
-------

    cargo build --release
    cargo test

The tests run against real `.kdenlive` files in `tests/data`, written by
Kdenlive rather than authored by hand. Three of them are consecutive saves
of one project. Two of those differ only in the order Kdenlive happened to
write the XML, with nothing actually edited, and the differ is expected to
report no changes for that pair.

See CONTRIBUTING.md if you want to send a patch.

Credit
------

Built against the Kdenlive project's own file format documentation, at
https://github.com/KDE/kdenlive/blob/master/dev-docs/fileformat.md, which is
the primary reference for everything in the parser.

Two details came from reading Kdenlive's source rather than the docs.
Current versions write media clips as `<chain>` rather than `<producer>`,
and markers are stored as a JSON array with frame positions rather than the
older `position:comment` form. A parser written from the documentation alone
finds no clips and no markers in a project saved by a recent Kdenlive.

The file format is based on MLT, https://www.mltframework.org.

License
-------

MIT. See LICENSE.
