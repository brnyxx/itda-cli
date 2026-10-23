# itda

**Just type an address after itda and your AI can use the site.**

Give itda one address. It learns what the site can do and tries it. It gives
your AI agent a short list of actions in plain words. Reads just work. Nothing
changes on the site until you say yes. It runs on your own computer, and it
works on the old intranet behind a login too.

It is for the sites your company runs and the services you have an account
on, not the web at large. [What it can and cannot do](https://github.com/brnyxx/itda-cli#what-it-can-and-cannot-do)
lists both, and what happens when a site changes.

<img src="https://raw.githubusercontent.com/brnyxx/itda-cli/main/assets/setup.gif" alt="itda setup on news.ycombinator.com: it finds a list of 30 records, a form and a feed, tries both reads, saves the site and adds itda to Claude Desktop, in fourteen seconds" width="100%">

```console
$ itda news.ycombinator.com
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

Ask your agent: *"use news-ycombinator to list what Hacker News shows"*. A site
behind a sign-in opens in your own browser. You sign in and use it as usual
while itda watches. What you type never leaves your computer.

## Why

Agents that drive a browser re-read every page and spend thousands of tokens on
each screenshot. They follow whatever the page tells them and break silently
when a layout changes. itda moves that work to a setup step. A site definition
says which actions exist, what they send, what a correct answer looks like and
whether they change anything. After that, every call is small, typed and
checked.

- **Checked.** A signed-out page or a redesigned site comes back as an error
  with what to do next, never as data.
- **Read-only by default.** Changes wait for a person to approve the exact
  request, in a terminal or in the agent app. No agent tool can approve.
- **Adaptive.** Reads can recover through browser fetch and rendering.
  Unresolved reads fail without locking the site; changes are never replayed.
- **Accountable.** A local log records every call, approval and change. Each
  entry seals the one before it, so edits show.
- **Local.** No server, account or telemetry. Secrets stay in your OS keyring.

## Where to go next

- [Quick start](quickstart.md): install, set up a site you use, and try the
  bundled sandbox.
- [Use it from an agent](agents.md): connect Claude, Cursor, VS Code or Codex.
- [Writing a site definition](SPEC.md): add your own site by hand.
