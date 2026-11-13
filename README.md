# majipdf

A PDF toolkit for Windows that works entirely offline. It compresses, merges, splits and
reorders PDFs, edits their text, makes scanned PDFs searchable, and converts them to
Word and Excel. The
whole thing is a single exe that you run without installing, and it doesn't need admin
rights.

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

On first launch majipdf unpacks its helpers (PDFium, Ghostscript, ONNX Runtime and the
OCR models) to
`%LOCALAPPDATA%\majipdf\runtime\` and checks them against hashes built into the exe. It
does this check on every launch and repairs the folder if anything was changed.

majipdf has no network code, and your documents never leave the computer. Two parts it
uses come from Microsoft and would report usage data by default, so majipdf turns that
off: ONNX Runtime (Accurate OCR) is loaded with its telemetry disabled, and the window
(WebView2) starts with its background network services switched off. WebView2's own
diagnostic data still follows the Windows diagnostic data setting, which an app can't
override. Word and Excel, used for PDF to Word and the second Excel method, follow your
own Office privacy settings.

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
  it. Useful when you've lost the original .docx. For scans, it can recognize the text
  first, so you get real text instead of pictures of pages.
- PDF to Excel: a built-in converter that doesn't need Office. The sheet is laid out
  like the PDF page, with the same column widths, merged header cells, borders and
  logos. Values are copied exactly as shown (`054` stays `054`), and you can switch
  numbers to real numbers if you need to calculate. If the result isn't right, there's
  a second method that uses Excel's own PDF import. For scans, it can recognize the
  text and the table lines first.
- Scan to text (OCR): adds an invisible text layer to scanned pages, so you can search,
  select and copy their text. Indonesian and English. Pages that already have text are
  kept as they are. Settings has two OCR quality levels: Accurate (the default, using
  PaddleOCR) reads noticeably better, especially table headers, and keeps the file size
  almost unchanged; Fast (Tesseract) is about 3 to 5 times quicker, which helps on long
  scans.
- Edit PDF: click a paragraph or line and change its text. It rewraps in place in the
  PDF's own font, and the text below moves to make room. If the font is missing a letter
  you typed, you can switch that paragraph to a similar font from your computer. Works
  best on PDFs made from Word; text in scans, repeated footers and unusual fonts stays
  locked. You can also:
  - make selected words bold, italic, bigger or another colour, or change a paragraph's
    font and alignment;
  - white out an area. What's under it is really removed, not just covered, so it
    can't be copied back out, and this works on scanned pages too. Comments and form
    fields in that area go too. If something can't be removed safely (a few unusual
    image types, for example), majipdf tells you and lets you decide whether a white
    cover is enough;
  - add text boxes anywhere on the page;
  - place a signature or stamp, either imported from a photo or scan (the white
    background is removed for you) or drawn with the mouse or a pen. Your signatures
    are kept on this computer only, ready for next time, and you can manage or delete
    them in Settings;
  - select existing images, lines and paragraphs to move, resize or delete them;
  - add a watermark (text such as DRAFT or RAHASIA, or an image) and headers or
    footers with page numbers ("Halaman 1 dari 10"), the date or the file name.

  Undo and redo work across everything, and the result is saved as `<name>_edited.pdf`.

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

- Filling in PDF forms.
- ZIP files as input for Compress. A folder works.
- A macOS version. The code builds for macOS and passes automated checks, but I don't
  own a Mac, so it hasn't been properly tested by hand yet.

This is an early release, and the version number reflects that. Bug reports and
suggestions are welcome in [Issues](https://github.com/majimawrks/majipdf/issues).

## Building from source

You need Node.js 20 or newer and Rust (stable, MSVC toolchain). The native helpers
aren't in the repo, so put them in `_tools/` first:

- `_tools/pdfium/bin/pdfium.dll` from
  [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) (Windows x64)
- `_tools/gs/bin/gswin64c.exe` and `gsdll64.dll` from
  [Ghostscript](https://www.ghostscript.com/releases/) 10.x (Windows 64-bit)
- `_tools/tessdata/eng.traineddata` and `ind.traineddata` from
  [tessdata_fast](https://github.com/tesseract-ocr/tessdata_fast)
- `_tools/paddle/onnxruntime.dll` from
  [ONNX Runtime](https://github.com/microsoft/onnxruntime/releases) 1.28.3 (win-x64), plus
  `ch_PP-OCRv5_det_mobile.onnx`, `latin_PP-OCRv5_rec_mobile.onnx` and `latin_v5.dict.txt`
  (PaddleOCR PP-OCRv5 models in ONNX form, from [RapidOCR](https://github.com/RapidAI/RapidOCR))

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
reading and page operations, Ghostscript for compression and Fast OCR, PaddleOCR on ONNX Runtime for Accurate OCR, rust_xlsxwriter for the Excel
converter, lopdf for editing text inside the page, and small embedded PowerShell scripts that drive Word and Excel over COM.

I built majipdf with a lot of help from AI coding tools (Claude and Codex). I reviewed
and tested every feature on real documents, but please report anything that looks off.

## License

majipdf's own code is MIT licensed, see [LICENSE](LICENSE). The release exe also
contains Ghostscript (AGPL-3.0) and PDFium (BSD) among others. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for details and source links.
