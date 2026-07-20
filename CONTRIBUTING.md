Contributing to cutback
=======================

Patches are welcome. This file describes what the project expects, so that
your change does not get held up over something mechanical.

Before you start
----------------

For a bug, open an issue with the Kdenlive version, the cutback version and
enough detail to reproduce it. If a project file is involved and you can
share it, that helps more than anything else, since almost every bug in this
tool is really a bug in reading some real file.

For a feature or a change to how something works, open an issue and describe
the approach before writing it. That costs you a comment and saves you the
risk of writing something that gets turned down for a reason you could not
have guessed.

Two things are deliberately out of scope. There is no support for other
editors, and there will not be until Premiere is worked on as its own
effort, because an abstraction invented before the second editor exists will
be the wrong abstraction. There is no merge command, because this is a tool
for one person on one machine.

Building and testing
--------------------

    cargo build
    cargo test
    cargo clippy --all-targets
    cargo fmt

All four are run in CI and all four should be clean before you open a pull
request. The install script is also exercised in CI, so if you change it,
check that `./install.sh --prefix /tmp/somewhere -y` still works and that
`--uninstall` removes what it installed.

Tests use real project files
----------------------------

The files in `tests/data` were written by Kdenlive, not by hand. This is not
incidental. The format documentation is incomplete in ways that matter, and
two of the three parser bugs found while building this tool would have been
invisible against invented input. Current Kdenlive writes media clips as
`<chain>` where the docs say `<producer>`, and stores markers as JSON with
frame positions where the docs describe `position:comment` in seconds.

If you are adding parser support for something, add a real file that
contains it, or edit one of the existing files in a single specific way and
say so in the test. Do not write a synthetic project file from scratch.

Some tests depend on exact values in those files, such as a particular blank
length or gain property. If you replace a sample file, expect to update
those and check that they still test what they claim to.

Code style
----------

Match what is already there. A few points that come up:

Comments explain why, not what. If the code already says it, the comment is
noise. The comments that earn their place in this codebase are the ones
recording a fact you cannot see locally, such as why the watcher keys on a
rename rather than a close, or why document versions are compared as floats.

Prefer making invalid states unrepresentable over checking for them later,
but do not build a generic abstraction for a single concrete case.

Avoid `unwrap` outside tests. Where it is genuinely infallible, say why in a
comment.

No emoji in code, comments or commit messages.

Two rules the parser follows that are worth keeping
---------------------------------------------------

Fail loudly rather than partially. If the parser meets something it does not
understand, it returns an error. A diff computed from a half understood file
misleads the person reading it, which is worse than showing them nothing.

Never write XML back. cutback reads the project file to describe it and
restores by writing back the exact bytes it stored earlier. It does not
edit, reformat or regenerate the document, which is what makes byte for byte
restores possible and avoids the namespace problem that makes projects with
track wide effects invalid to generic XML tools.

Commits and pull requests
-------------------------

One logical change per commit. Write the subject in the imperative, under
about seventy characters, with no trailing period.

    Add parser support for subtitle tracks

Keep it to a single line unless the reasoning genuinely needs a body, in
which case leave a blank line and write full sentences. No tool attribution
trailers.

In the pull request, say what the problem was and how you approached it.
Mention any tradeoff a reviewer would otherwise have to find on their own.
Plain paragraphs are fine, and there is no template to fill in.
