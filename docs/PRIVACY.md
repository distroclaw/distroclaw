# Privacy in DistroClaw Community

DistroClaw runs on your computer. Your chats, documents, attachments, settings and the AI model stay there. This page lists every case in which the app connects to the internet, and what is sent.

> Draft for legal review.

## What stays on your computer

Everything you create or open with DistroClaw is stored in its data folder, `~/Library/Application Support/com.distroclaw.app/` (on Windows `%LOCALAPPDATA%\com.distroclaw.app\`):

- conversations, projects, attachments and text extracted from your documents
- the folders you allowed, your skills and settings
- the AI models and the engine's log

The AI engine runs on your computer and accepts connections only from it. Nobody, including us, can see or recover this data. Deleting the data folder removes it (see [Uninstalling](../README.md#uninstalling)).

The app contains **no analytics, telemetry, crash reporting or advertising**.

On Windows, the app's window is drawn by Microsoft WebView2, a part of Windows. On its own, WebView2 would contact Microsoft in the background while the app runs. DistroClaw turns those background services off and gives WebView2 no route to the internet, and every Windows build is tested while running: the test fails if the app opens any connection to another computer. WebView2's own updates come through Windows and Microsoft Edge, not through DistroClaw.

## When the app goes online

| When | Who receives what | Can you avoid it? |
|---|---|---|
| **Downloading a model**, after you agree on the model setup screen | The DistroClaw model library, hosted by Cloudflare, Inc., receives your IP address, the app's network request and which model files are downloaded. | Yes, after the first model: you choose when to download more. The app needs one model to work. |
| **Online search**, only if you turn it on (Settings > General). It is **off** by default. | DuckDuckGo and Wikipedia receive the search terms the AI writes from your question, and your IP address. The websites in the results receive your IP address when the app reads them. If a page needs JavaScript, the app may open it in an installed Chrome, Edge, Chromium or Brave, with a temporary profile that is deleted afterwards. | Yes: leave online search off. |
| **Olymp research** (`/olymp`), only once an admin installed Olymp and while online search is on | Each source Olymp asks receives the search words the AI writes from your question, and your IP address: DuckDuckGo, Wikipedia, GDELT (news, off until you switch it on), Hacker News through Algolia, OpenAlex, arXiv, Crossref, Stack Exchange and GitHub, each one only for the kind of search it serves. Brave Search and Google receive them too, only if you saved your own key for them. The websites and document servers Olymp reads receive your IP address and a request that names Olymp (`DistroClaw-Olymp/0.1`). Olymp follows each site's robots.txt and pace. Your files never reach a source: once Olymp opens a local file or folder, it stops searching for the rest of that reply, and it does not search at all in a chat that holds your attachments. | Yes: leave online search off, or switch single sources off in Settings > Olymp. |
| **SearXNG and Crawl4AI on your computer**, only if you run them | Olymp sends search words to a SearXNG, and addresses of public pages to a Crawl4AI, at the addresses set in Settings > Olymp. These must be addresses on your computer (127.0.0.1). What these services then send to search engines and websites is under your control and their own settings. | Yes: don't run them, or set their addresses to off. |
| **Downloading DistroClaw** from GitHub | GitHub, Inc. receives your IP address and browser details, under GitHub's privacy statement. | — |

Links in answers open in your own browser only when you click them.

## The model library

Models come from the DistroClaw model library at `models.distroclaw.com` and `files.distroclaw.com`. We do not keep records of model downloads ourselves. Cloudflare, Inc., which hosts the library, processes each download request, including your IP address, to deliver the files, under its own privacy policy.

## Who is responsible

DistroClaw, Basel, Switzerland

Contact: [northstarlabs@googlegroups.com](mailto:northstarlabs@googlegroups.com)

For questions about your data, or to exercise your rights (access, correction, deletion, objection), contact us at the address above.
