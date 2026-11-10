# Olymp, the research agent

Olymp researches online across many sources at once and answers with numbered sources. It is an **add-on**: it does nothing until an admin installs it in **Settings > Olymp**, and uninstalling removes it again. Once installed, type `/olymp` in a chat, or choose Olymp in the message box. It needs **Online search** to be on (Search in the message box, or Settings > General).

## Install and uninstall

- **Install:** Settings > Olymp > **Install Olymp**. Olymp then appears in the skill list and the message box. It uses your local model; nothing else is downloaded.
- **More sources (optional):** with Docker Desktop installed and running, **Set up SearXNG and Crawl4AI** starts both on this computer (127.0.0.1 only) and sets their addresses. The first setup downloads about 2 GB of images.
- **Uninstall:** Settings > Olymp > **Uninstall Olymp** removes Olymp, its settings, search keys and everyone's source switches. If the app set up SearXNG and Crawl4AI, they are stopped and removed with their images. Your chats stay as they are.

Only an admin can install or uninstall Olymp. Until it is installed, `/olymp` answers that Olymp isn't installed, and nothing goes online for it.

What it does:

- searches the web, news, research papers and developer sources in one go, and merges the results (a page several sources found ranks higher; no site takes more than two places);
- finds public documents by keywords or file name: PDF, Word, Excel, PowerPoint and CSV; when a search for documents, news, papers or code finds nothing, the same words are tried on the web in the same step;
- opens results and reads them, documents included, in parts when they are long;
- works in steps: it searches, reads, checks facts in more than one source and searches again with other words;
- shows what it is doing while it works ("Olymp · Searching papers for …"), and every step as a row you can open;
- works with every model: models that can call tools research themselves; for models that cannot, the app plans the searches with the model, runs them and gives the model the results to answer from. If a model ends its research without an answer, Olymp asks it once more, without tools, to answer from what it found.

Answers cite their sources as [1], [2]. A number that points to no source is removed before the answer is saved.

## Sources

| Source | Used for | Needs |
|---|---|---|
| SearXNG on your computer | Web, news, papers, code; through it Google, Bing, Yahoo, Startpage, Brave, Qwant, Mojeek and more | You run it (see below) |
| DuckDuckGo | Web and code, public documents | — |
| Wikipedia (English, German, French, Italian) | Web | — |
| GDELT | News from around the world | Switched on by you: it starts off, as it is often busy and slow |
| Hacker News (Algolia) | Technology news | — |
| OpenAlex | Research papers, open-access PDFs | — |
| arXiv | Preprints, PDFs | — |
| Crossref | Published articles and reports | — |
| Stack Exchange | Programming questions | — |
| GitHub | Code repositories | — |
| Brave Search API | Web and news | Your key |
| Google Programmable Search | Web, public documents | Your key and search engine ID |

Each source receives the search words the AI writes from your question, and your IP address. Switch single sources off in **Settings > Olymp**. Olymp never scrapes Google, Bing, Yahoo or Quora directly: that is against their terms, and the app never works around bot checks. They are reached through your own SearXNG or the official APIs above; pages from sites like Quora are read where their robots.txt allows.

## Your files stay out of searches

- Olymp does all web research first. Once it opens a local file or folder, its web tools refuse for the rest of that reply.
- It does not search at all in a chat that holds attachments, because the model sees them and could put their words into a search. Start a new chat for the research.
- Only results of the current reply and links you wrote can be opened, so no page or file can make the model send text to an address of its choosing.

## Respecting websites

- robots.txt is read for every site (group `distroclaw-olymp`, else `*`) and checked before every request, redirects included; Crawl-delay is kept.
- One request at a time per site, at least a second apart. A site that answers 429 or 503 with Retry-After is left alone for that long; a wait over ten seconds means the page is skipped.
- Bot checks, logins and paywalls are respected, never worked around.
- Olymp names itself: `DistroClaw-Olymp/0.1 (+https://distroclaw.com/docs/)`.
- Each API's usage policy is kept as a pace (arXiv 3 s, GDELT 5 s, GitHub 6 s between requests); rate-limit answers pause the source.

## Limits per reply

6 searches, 6 pages or documents read, 15 MB per document (checked before downloading), 30 MB in total, 45 s per step, 12 model rounds and 10 minutes.

## SearXNG and Crawl4AI on your computer (optional)

[SearXNG](https://docs.searxng.org) is a metasearch engine: one query reaches many search engines. [Crawl4AI](https://docs.crawl4ai.com) renders pages that need JavaScript. Olymp works without them; with them it finds more and reads more pages.

With Docker Desktop running:

```sh
cd docs/olymp
SEARXNG_SECRET=$(openssl rand -hex 32) docker compose up -d
```

This starts SearXNG on `http://127.0.0.1:8888` (the address Olymp tries by default) and Crawl4AI on `http://127.0.0.1:11235`. Both are published on 127.0.0.1 only, so other computers on your network cannot reach them. Or let the app do it (Settings > Olymp > **Set up SearXNG and Crawl4AI**). If you start them yourself, then in **Settings > Olymp**:

1. Leave the SearXNG address empty (it uses `http://127.0.0.1:8888`).
2. Set the Crawl4AI address to `http://127.0.0.1:11235`. It stays off until you do.
3. Press **Test**. Both should say "Running".

The addresses must be written as 127.0.0.1 (or [::1]); anything else is refused. Only an admin can change them.

`searxng/settings.yml` switches on the JSON format Olymp needs and enables the engines listed above. Which engines you enable, and keeping to their terms, is up to you. To stop: `docker compose down`.

Crawl4AI loads the page and its resources in its own browser. Olymp only sends it pages it already read from a public address, asks it to respect robots.txt, and drops the result if the page ended up anywhere else.

## Search keys (optional)

- **Brave Search API:** create a key at brave.com/search/api and save it in Settings > Olymp.
- **Google Programmable Search:** a Custom Search JSON API key and a search engine ID (cx), if your Google account has access to that API.

Keys are stored in the app's local database and never shown again.

## Troubleshooting

| What you see | What to do |
|---|---|
| "Olymp searches online. Turn on Search, then send again." | Turn on Search in the message box. |
| "Olymp can't search the web in a chat that contains your files…" | Start a new chat for the research. |
| SearXNG: "Running, but its JSON format is off" | Add `json` to `search.formats` in SearXNG's settings.yml and restart it. |
| SearXNG: "Not running at this address" | Start it (`docker compose up -d`) or set the address to off. |
| A source shows "busy" or "paused" in a step | It asked to be left alone for a while; the other sources still answer. |

## For developers

Code: `src-tauri/src/olymp/`. Run the tests that need the internet or a running SearXNG, Crawl4AI or model with `cargo test olymp -- --ignored --nocapture`.
