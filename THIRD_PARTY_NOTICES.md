# Third-party notices

majipdf's own code is MIT licensed (see `LICENSE`). The release build `majipdf.exe`
also contains the components below. Their license texts are in the `licenses/` folder
of the release zip.

## Ghostscript 10.08.0

- Used for: Compress (and "Compress the result" in Merge), Scan to text (OCR)
- License: GNU Affero General Public License v3.0 (AGPL-3.0)
- Copyright (C) Artifex Software, Inc.
- How it's used: the unmodified `gswin64c.exe` and `gsdll64.dll` are embedded in
  `majipdf.exe`, unpacked to `%LOCALAPPDATA%\majipdf\runtime\`, and run as a separate
  program. majipdf does not link to Ghostscript.
- Source code: https://github.com/ArtifexSoftware/ghostpdl (tag `ghostpdl-10.08.0`),
  also at https://www.ghostscript.com/releases/
- License text: `licenses/ghostscript-AGPL-3.0.txt`

## Tesseract OCR 5.3.4 and Leptonica 1.84.1 (inside Ghostscript)

- Used for: Scan to text (OCR)
- Built into the official Ghostscript 10.08.0 Windows binary (`gsdll64.dll`) by Artifex;
  majipdf does not modify or link to them.
- Tesseract: Apache License 2.0, https://github.com/tesseract-ocr/tesseract
  (license text: `licenses/Apache-2.0.txt`)
- Leptonica: Leptonica License (BSD-2-Clause style), Copyright Leptonica,
  http://www.leptonica.org/ (license text: `licenses/leptonica-license.txt`)

## Tesseract language data (tessdata_fast: eng, ind)

- Used for: Scan to text (OCR), English and Indonesian
- License: Apache License 2.0, https://github.com/tesseract-ocr/tessdata_fast
- The unmodified `eng.traineddata` and `ind.traineddata` are embedded in `majipdf.exe`
  and unpacked next to Ghostscript.
- License text: `licenses/Apache-2.0.txt`

## PDFium (chromium build 8066)

- Used for: reading PDFs, thumbnails, Merge, Split, Organize, PDF to Excel
- License: BSD 3-Clause (PDFium, Copyright The PDFium Authors), plus the licenses of the
  libraries it includes: FreeType, libjpeg-turbo, OpenJPEG, Little CMS, libpng, zlib,
  ICU, Abseil, AGG, fast_float, LLVM libc, simdutf
- Binary package: pdfium-binaries by Benoit Blanchon (MIT),
  https://github.com/bblanchon/pdfium-binaries
- License texts: `licenses/pdfium/`

## Source Sans 3 (font) 5.3.0

- License: SIL Open Font License 1.1
- Copyright Adobe. Packaged by Fontsource (https://fontsource.org)

## Phosphor Icons 2.1.2

- License: MIT, Copyright (c) 2020 Phosphor Icons
- https://github.com/phosphor-icons/web

## Rust and JavaScript libraries

majipdf is built on Tauri, Svelte, pdfium-render, rust_xlsxwriter, lopdf, ttf-parser, image, zip, sha2 and
their dependencies. These are MIT, Apache-2.0, BSD or similarly permissive licensed. The
full dependency lists are in `src-tauri/Cargo.lock` and `package-lock.json`.

## Microsoft Office

PDF to Word and the "other method" of PDF to Excel drive the copy of Microsoft Word or
Excel already installed on the computer. Office is not included in or distributed with
majipdf.
