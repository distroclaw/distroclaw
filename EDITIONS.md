# Editions

DistroClaw comes in two editions:

- **DistroClaw Community** (this repository, Apache-2.0): a complete private AI assistant that runs on your own computer.
- **DistroClaw Pro**: a paid edition built on Community. It adds features that change files and help businesses. Its code is not public.

This page says where the line between them sits. That way you know before you start work whether a contribution belongs here.

## What Community includes

Everything one person needs to work with a local AI on their own computer, with no limits on folders, files or model size:

- Chat with streaming answers, Markdown, code highlighting and the model's reasoning
- Attachments: images, PDF, Word and text files
- Projects with their own instructions
- Asking questions about any number of folders, read-only, with sources
- Any model from the DistroClaw model library that your computer can run, model switching and context length
- Built-in skills, plus writing, importing and testing your own skills
- Online search (off by default; you turn it on)
- Settings, themes, the local database and everything else in this repository

## What Pro adds (not in this repository)

- Changing files, only after you approve each change: create, rename, move, organise, clean up, with undo and a change history
- Whole-computer access and terminal commands for an administrator at the desktop
- Skills for invoices, contracts and data tables
- Later, in a Business edition: shared access over a company network, with accounts, roles and an audit log

## How we decide where a feature goes

1. Does one person need it to use a private AI on their own computer? It goes in **Community**.
2. Is it a security fix, an accessibility, translation or platform improvement, or documentation? It always goes in **Community**.
3. Do comparable free tools offer it? It goes in **Community**; we do not charge for the basics.
4. Does it change the user's files or computer on their behalf, or save a professional hours of document work? It is a candidate for **Pro**.
5. Does it only matter once there is a team, an administrator or compliance? It is a candidate for **Business**.

## Our promises

- We will not move a Community feature into a paid edition.
- We will not add limits to Community to push people towards Pro.
- Community gets security fixes first, and from the same people.
- Before you build a larger feature, open an issue. A maintainer tells you which edition it belongs to before you write code, so your work is never wasted.

The Apache License lets anyone build anything on this code, including features that Pro has. We will not merge pull requests that add the Pro features listed above to this repository. If the community builds something better than ours, we will talk about making it part of Community.
