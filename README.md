# DistroClaw Community

A private AI assistant that runs on your Mac or Windows PC. Chat, ask questions about your documents and folders, and get help with writing. The AI model runs on your computer, so your chats and files stay there.

DistroClaw Community is free and open source under the [Apache License 2.0](LICENSE).

## Features

- **Chat** with streaming answers, Markdown, code highlighting and the model's reasoning.
- **Attachments:** add up to 10 images and 10 documents (PDF, Word, text and code files) to a message.
- **Your folders:** let DistroClaw read the folders you choose and answer questions about them, with sources. It cannot change your files.
- **Projects** group chats and give them their own instructions.
- **Skills** for summarising, translating, document questions and image analysis. You can also write and import your own.
- **Models:** DistroClaw suggests a model that fits your Mac. You can switch to any other model it can run and set the context length.
- **Online search** (off by default): turn it on when you want answers with current information.
- **Olymp** (`/olymp`, an add-on you install in Settings > Olymp; needs online search): researches across many sources at once (the web, news, research papers, developer sites and public PDF, Word and Excel files) and answers with numbered sources. It can also use a SearXNG and Crawl4AI you run on your Mac; see [docs/OLYMP.md](docs/OLYMP.md).
- **Works offline** once a model is downloaded. No account, no telemetry.

## Privacy

Everything you create or open stays in DistroClaw's data folder on your Mac. The app goes online only in two cases:

- to download a model you agreed to download from the DistroClaw model library
- to search the web, if you turn on online search (Olymp then also asks the news, paper and developer sources listed in the privacy notes)

[docs/PRIVACY.md](docs/PRIVACY.md) lists exactly what is sent in each case.

## Requirements

- macOS 14 or later on a Mac with Apple Silicon (M1 or newer). Intel Macs are not supported.
- Or Windows 10 (22H2) or Windows 11, 64-bit (x64). Windows on ARM is not supported. A separate graphics card (NVIDIA, AMD Radeon or Intel Arc) is optional: DistroClaw uses it through Vulkan when its driver supports that. Otherwise, and on PCs with only built-in graphics, the model runs on the processor, which is slower than on a Mac, especially with long documents.
- Free disk space for the model: 1.3 GB to 22 GB depending on the model; the recommended model needs 3 to 8 GB (about 8 GB on a Mac with 16 GB of memory).
- An internet connection for the first model download.

## Install

### Mac

1. Download `DistroClaw_macOS_arm64.dmg` from the [latest release](https://github.com/distroclaw/distroclaw/releases/latest).
2. Open it and drag **DistroClaw** to **Applications**.
3. Open DistroClaw from Applications.

**Test builds are not yet signed by Apple.** macOS will say it cannot verify the developer, and will not open the app the first time. To open it anyway:

1. Click **Done** in the message.
2. Open **System Settings > Privacy & Security** and scroll down to the DistroClaw message.
3. Click **Open Anyway** and confirm with your password or Touch ID.

You only need to do this once per version. Only open builds you downloaded from this repository's releases page.

### Windows

1. Download `DistroClaw-Setup.exe` from the [latest release](https://github.com/distroclaw/distroclaw/releases/latest).
2. Run it. It installs DistroClaw for your Windows user account, without administrator rights, into `%LOCALAPPDATA%\DistroClaw`, and adds it to the Start menu. It needs no internet connection.
3. Open DistroClaw from the Start menu.

**The installer is not yet code-signed.** Windows shows warnings for it:

- Microsoft Defender SmartScreen says "Windows protected your PC" and names no publisher. Click **More info**, check that the file is `DistroClaw-Setup.exe`, then click **Run anyway**.
- On a Windows 11 PC with Smart App Control turned on, Windows may block the installer and DistroClaw itself completely, with no way to run them anyway.
- Your antivirus software may also ask about it.

Only run installers you downloaded from this repository's releases page. To check that the file is genuine, compare its checksum with `SHA256SUMS.txt` on the release page:

```powershell
Get-FileHash "$HOME\Downloads\DistroClaw-Setup.exe" -Algorithm SHA256
```

## First run

1. DistroClaw checks your Mac's memory and suggests a model.
2. Agree to the download and wait until it finishes. The model is stored on your Mac.
3. Start chatting. To ask about your documents, add a folder in **Settings > Access**.

## Updating

DistroClaw does not update itself yet. Download the new release and replace the app in Applications. Your chats, settings and models are kept.

On Windows, run the new `DistroClaw-Setup.exe`. It replaces the app and keeps your chats, settings and models; if DistroClaw is open, the installer asks to close it first.

## Uninstalling

1. Quit DistroClaw and move it from Applications to the Trash.
2. To also remove your chats, settings and downloaded models, delete these folders:
   - `~/Library/Application Support/com.distroclaw.app/`
   - `~/Library/Logs/com.distroclaw.app/`

   In Finder, choose **Go > Go to Folder** and paste the path.

On Windows:

1. Open **Settings > Apps > Installed apps**, find **DistroClaw** and choose **Uninstall**.
2. The uninstaller removes the app, its Start menu entry and the AI engine. It keeps your chats, settings and downloaded models unless you tick **Delete the application data**. They are in `%LOCALAPPDATA%\com.distroclaw.app\`.

Uninstalling never touches your own files, including the folders you let DistroClaw read.

## Build from source

```sh
git clone https://github.com/distroclaw/distroclaw.git
cd distroclaw
npm ci
npm test
```

[docs/BUILDING.md](docs/BUILDING.md) explains the requirements, running the app in development and building an installer. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) gives an overview of the code, and [docs/MODELS.md](docs/MODELS.md) explains where the AI models come from.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) and [EDITIONS.md](EDITIONS.md) first. Every commit needs a sign-off (`git commit -s`). Please follow our [Code of Conduct](CODE_OF_CONDUCT.md).

Report security problems privately, as described in [SECURITY.md](SECURITY.md).

## Licence

DistroClaw Community is licensed under the [Apache License 2.0](LICENSE); see also [NOTICE](NOTICE). Third-party components and their licences are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and in the app under **Settings > About**.

"DistroClaw" and the DistroClaw logo are trademarks; see [TRADEMARKS.md](TRADEMARKS.md).
