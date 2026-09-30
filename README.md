# majipdf

A PDF toolkit for Windows that works entirely offline. It compresses, merges, splits and
reorders PDFs, and converts them to Word and Excel. The whole thing is a single exe that
you run without installing, and it doesn't need admin rights.

I wrote it for my coworkers. Most of us had gotten used to uploading work documents to
online PDF sites to shrink or convert them, which is a bad habit when the documents are
internal. majipdf does the same jobs on your own computer. It has no network code, so
your files never go anywhere.

The interface is in English and Bahasa Indonesia.

## Download

Get the latest `majipdf-<version>-win64.zip` from
[Releases](https://github.com/majimawrks/majipdf/releases), unzip it anywhere, and run
`majipdf.exe`.

It runs on Windows 11 (64-bit). Windows 10 with the WebView2 runtime should work too, but
I haven't tested it. PDF to Word needs Microsoft Word installed; everything else works
without Office.

On first launch majipdf unpacks two helpers, PDFium and Ghostscript, to
`%LOCALAPPDATA%\majipdf\runtime\` and checks them against hashes built into the exe. It
does this check on every launch and repairs the folder if anything was changed.

The exe isn't code-signed yet, so Windows SmartScreen or a company policy may warn you
or block it. If your IT team blocks it, that's the reason.

## What it does

- Compress: many files or a whole folder at once. Pick a preset (72, 150 or 300 DPI),
  or set a maximum size in MB and let it find the best setting that fits. Grayscale is
  optional. If a file can't get any smaller, the original is kept.
- Merge: combine PDFs in the order you drag them into. You can compress the result too.
- Split: every page, page ranges like `1-3, 5, 8-10`, or every N pages.
- Organize pages: rotate, reorder, delete, or insert pages from another PDF.
- PDF to Word: uses the Word already on your computer and keeps the layout as Word reads
  it. Useful when you've lost the original .docx.
- PDF to Excel: a built-in converter that doesn't need Office. The sheet is laid out
  like the PDF page, with the same column widths, merged header cells, borders and
  logos. Values are copied exactly as shown (`054` stays `054`), and you can switch
  numbers to real numbers if you need to calculate. If the result isn't right, there's
  a second method that uses Excel's own PDF import.

A few things apply to every tool:

- Your original file is never changed, and nothing gets overwritten. Results go next to
  the original (`report_compressed.pdf`) or into one folder, and a name that's taken
  becomes `report_compressed (2).pdf`.
- Password-protected PDFs can be unlocked inside the app.
- Before changing a digitally signed PDF, it warns you that the new file's signature
  won't be valid.
- Long jobs show progress and can be cancelled. If you switch to another window, the
  taskbar button flashes when the job is done.

## Not there yet

- OCR. Scanned pages come out as images in Word and as empty sheets in Excel.
- Editing and signing (white-out, text boxes, stamps).
- ZIP files as input for Compress. A folder works.
- A macOS version.

This is an early release, and the version number reflects that. Bug reports and
suggestions are welcome in [Issues](https://github.com/majimawrks/majipdf/issues).

## Building from source

You need Node.js 20 or newer and Rust (stable, MSVC toolchain). The native helpers
aren't in the repo, so put them in `_tools/` first:

- `_tools/pdfium/bin/pdfium.dll` from
  [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) (Windows x64)
- `_tools/gs/bin/gswin64c.exe` and `gsdll64.dll` from
  [Ghostscript](https://www.ghostscript.com/releases/) 10.x (Windows 64-bit)

Then:

```
npm install
npm run tauri dev                       # development
npm run tauri build -- --no-bundle      # single exe
```

The Rust build output goes to `D:\majipdf-target`, outside the project folder. Set
`MAJIPDF_TARGET_DIR` to use a different location. The release exe ends up in
`<target>\release\majipdf.exe`.

The version is computed at build time as `0.<features>.<commit count>`; see
`scripts/version.mjs`.

## How it's built

Tauri 2 with a Svelte 5 frontend. The backend is Rust: PDFium through pdfium-render for
reading and page operations, Ghostscript for compression, rust_xlsxwriter for the Excel
converter, and small embedded PowerShell scripts that drive Word and Excel over COM.

I built majipdf with a lot of help from AI coding tools (Claude and Codex). I reviewed
and tested every feature on real documents, but please report anything that looks off.

## License

majipdf's own code is MIT licensed, see [LICENSE](LICENSE). The release exe also
contains Ghostscript (AGPL-3.0) and PDFium (BSD) among others. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for details and source links.
