# Security policy

## Reporting a vulnerability

Please report security problems **privately**. Use the repository's **Security** tab, then **Report a vulnerability** (GitHub private vulnerability reporting). Do not open a public issue, discussion or pull request for a vulnerability.

Include what you found, how to reproduce it, the version (Settings > About) and what an attacker could do with it. We aim to confirm your report within 5 working days, keep you updated, and credit you in the release notes unless you prefer otherwise.

## Supported versions

Security fixes go into the latest release. DistroClaw does not update itself yet, so please install new releases when they come out.

## How DistroClaw protects you

These are the boundaries we consider security-relevant. A way around any of them is a vulnerability:

- **Local AI engine:** the engine runs on your computer and listens only on the loopback interface (127.0.0.1). Other computers cannot reach it.
- **No telemetry:** the app sends no analytics, crash reports or usage data.
- **Network use:** the app goes online only to download a model you agreed to download from the DistroClaw model library, and, if you turn on online search, to search the web and read the pages it finds.
- **Folder access:** the AI can only read inside folders you added in Settings > Access. Hidden files, the app's own data and protected system locations stay out of reach, even through symbolic links.
- **Online search:** pages are fetched only from public internet addresses, never from your local network, with size and time limits.
- **Olymp research:** the same public-address rule applies to every page and document Olymp reads, redirects included. Olymp respects robots.txt (checked on every hop), keeps each site's pace and Retry-After, and never works around bot checks or logins. Documents are size-checked before they are downloaded; spreadsheets are read in a separate process with time and memory limits, like PDFs. Once a reply has opened a local file or folder, Olymp's web tools refuse for the rest of that reply, and Olymp does not run in a chat that holds attachments.
- **Olymp is an add-on:** it does nothing until an admin installs it. The app only runs Docker when an admin sets up or removes Olymp's services, with fixed commands, in the app's own data folder.
- **Local research services:** SearXNG and Crawl4AI addresses must be loopback addresses (127.0.0.1 or [::1]); they are checked when saved and before every request, the client never follows their redirects, and only an admin can change them. Crawl4AI only receives pages already read from a public address, and its result is dropped if the page ended up anywhere else. A Crawl4AI you run loads the page's own resources itself; run it only on this computer and keep its port on 127.0.0.1.
- **Window content:** the app's window cannot load remote scripts or images. Links open in your browser.

Search API keys you save for Olymp (Brave Search, Google) are stored in the local database, which is not encrypted, and are never shown again in the app.

Not in scope:

- The answers a model gives; AI output can be wrong.
- Attacks that need an administrator account or physical access to an unlocked computer.
- Weaknesses in the third-party engine or models that do not affect DistroClaw's boundaries. Please report those upstream as well.
