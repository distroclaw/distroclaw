# Changelog

All notable changes to DistroClaw Community are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-11-10

### Added
- Olymp (`/olymp`), a research agent for online search. It comes with the app as an add-on that an admin installs and uninstalls in Settings > Olymp. It searches the web, news, research papers and developer sources at once, finds public PDF, Word, Excel, PowerPoint and CSV files, reads them, and answers with numbered sources. It works with every model, shows its progress in the chat, follows robots.txt and each site's pace, and can use a SearXNG and Crawl4AI running on your Mac, which the app can set up with Docker. Uninstalling removes its settings, keys and services.
- The Agent (`/agent`) works through an open task step by step and, while Online search is on, searches and reads web pages itself, with numbered sources.
- Excel, OpenDocument spreadsheet and PowerPoint files can be read from allowed folders.

### Fixed
- Web pages whose scripts or forms mention a captcha, such as every Wikipedia article, are no longer mistaken for bot checks and skipped.

### Security
- Olymp never searches the web once a reply has opened a local file or folder, nor in a chat that holds attachments, so your files cannot end up in a search.

## [0.1.0] - 2026-11-06

### Added
- First public release of DistroClaw Community under the Apache License 2.0.
- Private AI chat that runs on your Mac, with answers from the folders you allow and from attached files.
- Projects with their own instructions, custom skills, and optional online search (off by default).
- Model setup that recommends a Qwen 3.5 or Gemma 4 model for your Mac and downloads it from the DistroClaw model library after you agree.
- Settings > Model to switch, remove and see the licence of installed models; Settings > About lists the open-source licences.
