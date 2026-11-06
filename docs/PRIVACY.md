# Privacy in DistroClaw Community

DistroClaw runs on your computer. Your chats, documents, attachments, settings and the AI model stay there. This page lists every case in which the app connects to the internet, and what is sent.

> Draft for legal review.

## What stays on your computer

Everything you create or open with DistroClaw is stored in its data folder, `~/Library/Application Support/com.distroclaw.app/`:

- conversations, projects, attachments and text extracted from your documents
- the folders you allowed, your skills and settings
- the AI models and the engine's log

The AI engine runs on your computer and accepts connections only from it. Nobody, including us, can see or recover this data. Deleting the data folder removes it (see [Uninstalling](../README.md#uninstalling)).

The app contains **no analytics, telemetry, crash reporting or advertising**.

## When the app goes online

| When | Who receives what | Can you avoid it? |
|---|---|---|
| **Downloading a model**, after you agree on the model setup screen | The DistroClaw model library, hosted by Cloudflare, Inc., receives your IP address, the app's network request and which model files are downloaded. | Yes, after the first model: you choose when to download more. The app needs one model to work. |
| **Online search**, only if you turn it on (Settings > General). It is **off** by default. | DuckDuckGo and Wikipedia receive the search terms the AI writes from your question, and your IP address. The websites in the results receive your IP address when the app reads them. If a page needs JavaScript, the app may open it in an installed Chrome, Edge, Chromium or Brave, with a temporary profile that is deleted afterwards. | Yes: leave online search off. |
| **Downloading DistroClaw** from GitHub | GitHub, Inc. receives your IP address and browser details, under GitHub's privacy statement. | — |

Links in answers open in your own browser only when you click them.

## The model library

Models come from the DistroClaw model library at `models.distroclaw.com` and `files.distroclaw.com`. We do not keep records of model downloads ourselves. Cloudflare, Inc., which hosts the library, processes each download request, including your IP address, to deliver the files, under its own privacy policy.

## Who is responsible

DistroClaw, Basel, Switzerland

Contact: [northstarlabs@googlegroups.com](mailto:northstarlabs@googlegroups.com)

For questions about your data, or to exercise your rights (access, correction, deletion, objection), contact us at the address above.
