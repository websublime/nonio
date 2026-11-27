---
name: nonio
description: Draw a checked, interactive map that answers how something in this codebase works ("what happens when a user signs up?", "how does this page reach the API?", "what did the last commit change?"). Use when the user wants to understand a flow across files, asks how parts connect, what someone goes through in the app, how a change would look before it is made, or asks for a map, diagram or walkthrough of the code. Every box and arrow on the map is checked against the code.
---

# nonio

nonio turns a question about this repo into a map the person explores in their browser: areas of the app, the steps
in order, labelled arrows, and red dashed arrows where something that should be connected isn't. A deterministic
program checks every claim against the code before it is shown. On the map they can take a walkthrough, select a box
or an arrow and ask about it, and approve fixes that nonio applies and checks again.

## When to use it

- The user asks how something works across several files: "what happens when…", "how does X reach Y", "where does Z
  get triggered".
- The user is about to change code they don't understand yet and wants to see it first.
- The user asks how a change would look before it is made ("how would we add…", "plan this"): the map shows the code
  as it is, with the proposal drawn on top.
- The user asks what a change did ("what did you just change?", "what did the last commit do?").
- The user asks what someone sees or goes through in the app (a user journey).
- The user asks for a map, diagram, graph or walkthrough of part of the code.

Don't use it for a question about a single line or function; answer that directly.

## How to run it

You have nonio's MCP tools (`nonio_map`, `nonio_submit` and the others). Call `nonio_map` with the user's question in
their own words. nonio's server tells you, when it connects, how this agent works with it: whether to pass `repo`,
how to carry on, and how questions asked about the map reach you. Follow those instructions. While you work on a nonio
request, read the code with read-only tools, don't edit files, and keep what you say in the chat to a sentence or two:
the map does the explaining. `nonio_open` opens saved maps (they live in `.nonio/maps/`).

## When there is no `nonio_map` tool

The plugin is installed but nonio's server didn't start. The plugin starts it with
`npx -y @websublime/nonio@0.0.1 mcp`, which needs Node and npm on the machine (the server itself is a native binary
inside the npm package).

1. Run `node --version` and `npx --version`. If either is missing, say so in a sentence and offer to install Node
   with what the machine already uses (a version manager, Homebrew, winget). Do it only if the person agrees, and
   avoid anything that asks for an administrator password.
2. Run `npx -y @websublime/nonio@0.0.1 --version`. The first run downloads the package, and an agent may give up
   waiting for a slow first start. If it prints `nonio 0.0.1`, tell the person to restart nonio's MCP server in their
   agent (or reload the window), then ask again.
3. If it fails with `E404`, `E401` or `E403` for `@websublime/nonio`, npm can't reach the registry that holds nonio.
   Inside a company that is usually the internal registry: the person's npm configuration needs a line such as
   `@websublime:registry=<their registry URL>` and a sign-in for it. Tell them that in plain words. Don't change their
   npm configuration or sign in for them.
4. If it fails with anything else, tell them the error in plain words. Don't install anything else.

## Notes

- nonio reads the code and never changes it, except for a fix the person approves on the map.
- It never reads secret files (`.env`, keys) and only listens on 127.0.0.1.
- It uses the model of the agent it runs in, so it needs no API key.
