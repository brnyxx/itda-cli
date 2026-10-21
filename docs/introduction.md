# itda

**The websites you use, handed to your AI. Safely.**

Give itda one address. It learns what the site can do, tries it, and hands
your AI agent a short list of actions in plain words. Reads just work.
Nothing changes on the site until you say yes. It runs on your own computer,
and it works on the old intranet behind a login too.

<img src="https://raw.githubusercontent.com/brnyxx/itda-cli/main/assets/setup.gif" alt="itda setup on news.ycombinator.com: it finds a list of 30 records, a form and a feed, tries both reads, saves the site and adds itda to Claude Desktop, in fourteen seconds" width="100%">

```console
$ itda
itda hands the websites you use to your AI agent. Paste the address of a site you use and press Enter. No address in mind? Type github, npm, crates-io or pypi. Enter alone shows the command list.
Address: news.ycombinator.com
1/5  Looking at news.ycombinator.com...
    The page opens without signing in.
    Found: a list of 30 records, a form, a feed.
2/5  What agents will be able to do:
    read   home         List what Hacker News shows  (inputs: p)
    read   feed         Read the RSS feed
3/5  Trying every read once...
    ✓ home         30 records
    ✓ feed         ok
4/5  Saved as 'news-ycombinator'.
    Connect Claude Desktop? [Y/n]
5/5  Adding itda to Claude Desktop...
```

Then ask your agent: *"use news-ycombinator to list what Hacker News shows"*.
A site behind a sign-in opens in your own browser instead; you sign in and use
it as usual while itda watches, and what you type never leaves your computer.

## Why

Agents that drive a browser re-read every page, spend thousands of tokens on
each screenshot, follow whatever the page tells them, and break silently when
a layout changes. itda moves that work to a setup step. A site definition says
which actions exist, what they send, what a correct answer looks like and
whether they change anything. After that, every call is small, typed and
checked.

- **Checked.** A signed-out page or a redesigned site comes back as an error
  with what to do next, never as data.
- **Read-only by default.** Changes wait for a person to approve the exact
  request, in a terminal or in the agent app. No agent tool can approve.
- **Honest.** itda says who it is, never imitates a browser, and stops when a
  site refuses it.
- **Accountable.** Every call, approval and change goes into a local log
  where each entry seals the one before it, so edits show.
- **Local.** No server, account or telemetry. Secrets stay in your OS keyring.

## Where to go next

- [Quick start](quickstart.md): install, set up a site you use, and try the
  bundled sandbox.
- [Use it from an agent](agents.md): connect Claude, Cursor, VS Code or Codex.
- [Writing a site definition](SPEC.md): add your own site by hand.
