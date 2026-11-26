
from pathlib import Path
import csv
import json
import shutil
import subprocess
import zipfile
import html
import sys

# Корпус зберігається поза репозиторієм Sonne
ROOT = (Path.cwd() / "../sonne-benchmark-corpus").resolve()
SRC = ROOT / "source"
GT = ROOT / "ground-truth"
RES = ROOT / "results"

for directory in (SRC, GT, RES):
    directory.mkdir(parents=True, exist_ok=True)


def write(path, content):
    path.write_text(content, encoding="utf-8")


def save_json(name, data):
    write(GT / name, json.dumps(data, ensure_ascii=False, indent=2))


def make_zip(path, files):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in files.items():
            archive.writestr(name, content)


# --------------------------------------------------
# 08. SEMANTIC HTML
# --------------------------------------------------

write(SRC / "08-semantic.html", """<!DOCTYPE html>
<html lang="uk">
<head><meta charset="UTF-8"><title>Звіт Sonne</title></head>
<body>
<header>
<h1>Звіт про тестування Sonne</h1>
<p>Контрольний документ для перевірки витягування.</p>
</header>
<main>
<section>
<h2>Результати</h2>
<p>Українські символи: ґ, є, і, ї.</p>
<ul>
<li>Перший контрольний пункт</li>
<li>Другий контрольний пункт</li>
<li>Третій контрольний пункт</li>
</ul>
</section>
<section>
<h2>Таблиця показників</h2>
<table>
<tr><th>Формат</th><th>Кількість</th><th>Статус</th></tr>
<tr><td>PDF</td><td>12</td><td>Готовий</td></tr>
<tr><td>DOCX</td><td>8</td><td>Очікує</td></tr>
</table>
</section>
<p><a href="https://example.org">Тестове посилання</a></p>
</main>
<footer>Кінець документа. Версія 1.0.</footer>
</body>
</html>""")

save_json("08-semantic.json", {
    "headings": [
        "Звіт про тестування Sonne",
        "Результати",
        "Таблиця показників"
    ],
    "list_items": [
        "Перший контрольний пункт",
        "Другий контрольний пункт",
        "Третій контрольний пункт"
    ],
    "table": [
        ["Формат", "Кількість", "Статус"],
        ["PDF", "12", "Готовий"],
        ["DOCX", "8", "Очікує"]
    ]
})


# --------------------------------------------------
# 09. MARKDOWN / TXT
# --------------------------------------------------

markdown = """# Контрольний документ Sonne

## 1. Український текст

Символи: ґ, є, і, ї.
Апострофи: об'єкт, п'ять, комп'ютер.

## 2. English text

The quick brown fox jumps over the lazy dog.

## 3. Списки

- Перший елемент
- Другий елемент
  - Вкладений елемент
- Третій елемент

## 4. Числа

- Ціна: 1 299,99 грн
- Відсоток: 17.5%
- Математика: 2 + 2 = 4
- Символи: © ® € £ ¥

## 5. Код

```typescript
interface Document {
  id: string;
  title: string;
  pages: number;
}
```

## 6. Кінець

END-OF-DOCUMENT-SONNE-2026
"""

write(SRC / "09-text-and-markdown.md", markdown)
write(SRC / "09-text-and-markdown.txt", markdown)

save_json("09-text-and-markdown.json", {
    "required_strings": [
        "ґ, є, і, ї",
        "об'єкт, п'ять, комп'ютер",
        "The quick brown fox jumps over the lazy dog.",
        "Вкладений елемент",
        "1 299,99 грн",
        "END-OF-DOCUMENT-SONNE-2026"
    ],
    "code_language": "typescript"
})


# --------------------------------------------------
# 10. CSV — 120 ROWS
# --------------------------------------------------

csv_path = SRC / "10-data.csv"

names = ["Олена", "Ігор", "Марія", "John", "Ґанна"]
cities = ["Київ", "Львів", "Харків", "Одеса", "Хмельницький"]
comments = [
    "Звичайне значення",
    "Текст із комою, всередині",
    'Рядок із "цитатою"',
    "English text",
    "Українські символи: ґ, є, і, ї"
]

with csv_path.open("w", encoding="utf-8-sig", newline="") as f:
    writer = csv.writer(f)
    writer.writerow([
        "id", "name", "city", "amount",
        "date", "category", "active", "comment"
    ])

    for i in range(1, 121):
        writer.writerow([
            i,
            names[(i - 1) % len(names)],
            cities[(i - 1) % len(cities)],
            round(i * 137.29, 2),
            f"2026-{(i % 12) + 1:02d}-15",
            ["A", "B", "C"][i % 3],
            i % 2 == 0,
            comments[(i - 1) % len(comments)]
        ])

save_json("10-data.json", {
    "columns": [
        "id", "name", "city", "amount",
        "date", "category", "active", "comment"
    ],
    "data_rows": 120,
    "encoding": "UTF-8 with BOM",
    "required_values": comments
})


# --------------------------------------------------
# DOCX — VALID OOXML DOCUMENTS
# --------------------------------------------------

def make_docx(path, title, paragraphs):
    body = f"""
    <w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr>
    <w:r><w:t>{title}</w:t></w:r></w:p>
    """

    for text in paragraphs:
        body += f"""
        <w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>
        """

    document = f"""<?xml version="1.0" encoding="UTF-8"?>
    <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
    <w:body>{body}
    <w:sectPr/>
    </w:body></w:document>"""

    files = {
        "[Content_Types].xml": """<?xml version="1.0"?>
        <Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
        <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
        <Default Extension="xml" ContentType="application/xml"/>
        <Override PartName="/word/document.xml"
        ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
        </Types>""",

        "_rels/.rels": """<?xml version="1.0"?>
        <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId1"
        Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
        Target="word/document.xml"/>
        </Relationships>""",

        "word/document.xml": document
    }

    make_zip(path, files)


make_docx(
    SRC / "04-structured.docx",
    "Структурований звіт Sonne",
    [
        "1. Призначення документа",
        "Це контрольний документ українською та англійською мовами.",
        "2. Структура",
        "Заголовки, абзаци та контрольні дані.",
        "Формат | Кількість | Статус",
        "PDF | 12 | Готовий",
        "DOCX | 8 | Очікує",
        "3. Списки",
        "Перший пункт",
        "Другий пункт",
        "Вкладений пункт",
        "END-OF-DOCX-SONNE-2026"
    ]
)

make_docx(
    SRC / "05-images.docx",
    "Документ із контрольними ілюстраціями",
    [
        f"Розділ {i}: IMAGE-{i:02d}. Підпис до контрольного зображення."
        for i in range(1, 7)
    ]
)

save_json("04-structured.json", {
    "required_strings": [
        "Структурований звіт Sonne",
        "1. Призначення документа",
        "2. Структура",
        "END-OF-DOCX-SONNE-2026"
    ]
})

save_json("05-images.json", {
    "image_markers": [f"IMAGE-{i:02d}" for i in range(1, 7)],
    "note": "Це текстові маркери, а не вбудовані зображення."
})


# --------------------------------------------------
# XLSX — MULTI-SHEET WORKBOOK
# --------------------------------------------------

def col_name(n):
    result = ""
    while n:
        n, rem = divmod(n - 1, 26)
        result = chr(65 + rem) + result
    return result


sheets = {
    "Summary": [
        ["Показник", "Значення"],
        ["Кількість записів", 120],
        ["Загальна сума", "=SUM(Sales!D2:D121)"]
    ],
    "Sales": [
        ["ID", "Ім'я", "Місто", "Сума"],
        *[
            [
                i,
                names[(i - 1) % 5],
                cities[(i - 1) % 5],
                round(i * 137.29, 2)
            ]
            for i in range(1, 121)
        ]
    ],
    "Expenses": [
        ["Категорія", "Сума"],
        ["Оренда", 12000],
        ["Маркетинг", 4500],
        ["Інфраструктура", 3200]
    ],
    "Forecast": [
        ["Місяць", "Дохід", "Витрати", "Прибуток"],
        *[
            [f"Місяць {i}", 10000 + i * 500, 6000 + i * 200,
             f"=B{i+1}-C{i+1}"]
            for i in range(1, 13)
        ]
    ]
}

workbook = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>'
]

rels = []
xlsx_files = {}

for index, (name, rows) in enumerate(sheets.items(), 1):
    workbook.append(
        f'<sheet name="{name}" sheetId="{index}" r:id="rId{index}"/>'
    )
    rels.append(
        f'<Relationship Id="rId{index}" '
        'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" '
        f'Target="worksheets/sheet{index}.xml"/>'
    )

    sheet_xml = ['<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>']

    for row_index, row in enumerate(rows, 1):
        sheet_xml.append(f'<row r="{row_index}">')

        for column_index, value in enumerate(row, 1):
            ref = f"{col_name(column_index)}{row_index}"

            if isinstance(value, str) and value.startswith("="):
                sheet_xml.append(
                    f'<c r="{ref}"><f>{value[1:]}</f></c>'
                )
            elif isinstance(value, (int, float)):
                sheet_xml.append(
                    f'<c r="{ref}" t="n"><v>{value}</v></c>'
                )
            else:
                escaped = html.escape(str(value))
                sheet_xml.append(
                    f'<c r="{ref}" t="inlineStr"><is><t>{escaped}</t></is></c>'
                )

        sheet_xml.append('</row>')

    sheet_xml.append('</sheetData></worksheet>')
    xlsx_files[f"xl/worksheets/sheet{index}.xml"] = "".join(sheet_xml)

workbook.append('</sheets></workbook>')

xlsx_files["xl/workbook.xml"] = "".join(workbook)
xlsx_files["xl/_rels/workbook.xml.rels"] = (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    + "".join(rels) + '</Relationships>'
)

xlsx_files["_rels/.rels"] = (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    '<Relationship Id="rId1" '
    'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" '
    'Target="xl/workbook.xml"/></Relationships>'
)

xlsx_files["[Content_Types].xml"] = (
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Override PartName="/xl/workbook.xml" '
    'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
    '</Types>'
)

make_zip(SRC / "06-multisheet.xlsx", xlsx_files)

save_json("06-multisheet.json", {
    "sheets": list(sheets.keys()),
    "sales_rows": 120,
    "formulas": ["SUM(Sales!D2:D121)", "B2-C2"]
})


# --------------------------------------------------
# PPTX — 8 TEXT SLIDES
# --------------------------------------------------

slides = {}
slide_ids = []
slide_rels = []

for i in range(1, 9):
    slide_ids.append(f'<p:sldId id="{255+i}" r:id="rId{i}"/>')
    slide_rels.append(
        f'<Relationship Id="rId{i}" '
        'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" '
        f'Target="slides/slide{i}.xml"/>'
    )

    texts = [
        f"Слайд {i}: Огляд Sonne",
        f"Контрольний текст слайда {i}",
        f"Українська та English content SONNE-SLIDE-{i:02d}"
    ]

    shapes = ""
    for j, text in enumerate(texts, 2):
        shapes += f"""
        <p:sp>
        <p:nvSpPr><p:cNvPr id="{j}" name="Text {j}"/>
        <p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p>
        <a:r><a:rPr lang="uk-UA"/><a:t>{html.escape(text)}</a:t></a:r>
        </a:p></p:txBody>
        </p:sp>
        """

    slides[f"ppt/slides/slide{i}.xml"] = f"""<?xml version="1.0"?>
    <p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
    xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
    <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvSpPr/>
    <p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>
    {shapes}
    </p:spTree></p:cSld></p:sld>"""

slides["ppt/presentation.xml"] = (
    '<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" '
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
    '<p:sldIdLst>' + "".join(slide_ids) + '</p:sldIdLst>'
    '<p:sldSz cx="9144000" cy="5143500"/></p:presentation>'
)

slides["ppt/_rels/presentation.xml.rels"] = (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    + "".join(slide_rels) + '</Relationships>'
)

slides["_rels/.rels"] = (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    '<Relationship Id="rId1" '
    'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" '
    'Target="ppt/presentation.xml"/></Relationships>'
)

content_types = [
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">',
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>',
    '<Default Extension="xml" ContentType="application/xml"/>',
    '<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>'
]

for i in range(1, 9):
    content_types.append(
        f'<Override PartName="/ppt/slides/slide{i}.xml" '
        'ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>'
    )

content_types.append('</Types>')
slides["[Content_Types].xml"] = "".join(content_types)

make_zip(SRC / "07-visual-slides.pptx", slides)

save_json("07-visual-slides.json", {
    "slides": 8,
    "required_strings": [
        f"SONNE-SLIDE-{i:02d}" for i in range(1, 9)
    ],
    "note": "Текстові слайди без вбудованих ілюстрацій."
})


# --------------------------------------------------
# EPUB
# --------------------------------------------------

epub_path = SRC / "11-book.epub"

with zipfile.ZipFile(epub_path, "w") as archive:
    archive.writestr(
        "mimetype",
        "application/epub+zip",
        compress_type=zipfile.ZIP_STORED
    )

    archive.writestr(
        "META-INF/container.xml",
        """<?xml version="1.0"?>
        <container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
        <rootfiles><rootfile full-path="OEBPS/content.opf"
        media-type="application/oebps-package+xml"/></rootfiles></container>"""
    )

    manifest = []
    spine = []

    for i in range(1, 7):
        name = f"chapter{i}.xhtml"
        manifest.append(
            f'<item id="c{i}" href="{name}" media-type="application/xhtml+xml"/>'
        )
        spine.append(f'<itemref idref="c{i}"/>')

        archive.writestr(
            f"OEBPS/{name}",
            f"""<?xml version="1.0" encoding="utf-8"?>
            <html xmlns="http://www.w3.org/1999/xhtml">
            <head><title>Розділ {i}</title></head>
            <body>
            <h1>Розділ {i}. Тестування Sonne</h1>
            <p>Український текст та English text.</p>
            <p>BOOK-CHAPTER-{i:02d}-END</p>
            </body></html>"""
        )

    archive.writestr(
        "OEBPS/content.opf",
        """<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf"
        version="3.0" unique-identifier="uid">
        <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <dc:identifier id="uid">sonne-benchmark-epub</dc:identifier>
        <dc:title>Контрольна книга Sonne</dc:title>
        <dc:language>uk</dc:language>
        <dc:creator>Benchmark Generator</dc:creator>
        </metadata>
        <manifest>""" + "".join(manifest) +
        """</manifest><spine>""" + "".join(spine) +
        """</spine></package>"""
    )

save_json("11-book.json", {
    "title": "Контрольна книга Sonne",
    "language": "uk",
    "chapters": 6,
    "required_strings": [
        f"BOOK-CHAPTER-{i:02d}-END" for i in range(1, 7)
    ]
})


# --------------------------------------------------
# NESTED ZIP
# --------------------------------------------------

inner_one = ROOT / "_inner-one.zip"
inner_two = ROOT / "_inner-two.zip"

make_zip(inner_one, {
    "data.csv": "id,назва\n1,Тест\n2,Перевірка\n",
    "page.html": "<html><body><h1>NESTED-HTML-SONNE</h1></body></html>"
})

make_zip(inner_two, {
    "notes.md": "# Вкладений архів\nNESTED-MD-SONNE"
})

with zipfile.ZipFile(SRC / "12-nested-archive.zip", "w", zipfile.ZIP_DEFLATED) as archive:
    archive.writestr("readme.txt", "ARCHIVE-ROOT-SONNE")
    archive.write(inner_one, "level1/inner-one.zip")
    archive.write(inner_two, "level1/deeper/inner-two.zip")

inner_one.unlink()
inner_two.unlink()

save_json("12-nested-archive.json", {
    "nested_archives": 2,
    "expected_paths": [
        "readme.txt",
        "level1/inner-one.zip",
        "level1/deeper/inner-two.zip"
    ]
})


# --------------------------------------------------
# PDF GENERATION WITH macOS SWIFT
# --------------------------------------------------

swift_code = r'''
import AppKit
import CoreText
import CoreGraphics
import Foundation

let output = URL(fileURLWithPath: CommandLine.arguments[1])

func generatePDF(_ name: String, scanned: Bool, pages: Int) {
    let url = output.appendingPathComponent(name) as CFURL
    var rect = CGRect(x: 0, y: 0, width: 595, height: 842)

    guard let context = CGContext(url as CFURL, mediaBox: &rect, nil) else {
        fatalError("Cannot create PDF")
    }

    for page in 1...pages {
        context.beginPDFPage(nil)

        if scanned {
            let rep = NSBitmapImageRep(
                bitmapDataPlanes: nil,
                pixelsWide: 1190,
                pixelsHigh: 1684,
                bitsPerSample: 8,
                samplesPerPixel: 4,
                hasAlpha: true,
                isPlanar: false,
                colorSpaceName: .deviceRGB,
                bytesPerRow: 0,
                bitsPerPixel: 0
            )!

            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)!

            NSColor.white.setFill()
            NSBezierPath(rect: NSRect(x: 0, y: 0, width: 1190, height: 1684)).fill()

            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 28),
                .foregroundColor: NSColor.black
            ]

            let lines = [
                "Контрольний документ Sonne",
                "Сторінка \(page)",
                "Українські символи: ґ, є, і, ї",
                "English text for OCR testing",
                "PDF-PAGE-\(page)-SONNE-2026"
            ]

            var y: CGFloat = 1500
            for line in lines {
                NSString(string: line).draw(
                    at: NSPoint(x: 90, y: y),
                    withAttributes: attrs
                )
                y -= 130
            }

            NSGraphicsContext.restoreGraphicsState()

            context.draw(rep.cgImage!, in: CGRect(x: 0, y: 0, width: 595, height: 842))
        } else {
            let text = """
            Контрольний документ Sonne

            Сторінка \(page)

            Українські символи: ґ, є, і, ї.

            English text for document extraction.

            PDF-PAGE-\(page)-SONNE-2026
            """

            let attributed = NSAttributedString(
                string: text,
                attributes: [
                    .font: NSFont.systemFont(ofSize: 16),
                    .foregroundColor: NSColor.black
                ]
            )

            let setter = CTFramesetterCreateWithAttributedString(attributed)
            let path = CGMutablePath()
            path.addRect(CGRect(x: 45, y: 45, width: 505, height: 752))
            let frame = CTFramesetterCreateFrame(
                setter,
                CFRange(location: 0, length: 0),
                path,
                nil
            )
            CTFrameDraw(frame, context)
        }

        context.endPDFPage()
    }

    context.closePDF()
}

generatePDF("01-digital-text.pdf", scanned: false, pages: 6)
generatePDF("02-scanned.pdf", scanned: true, pages: 4)
generatePDF("03-mixed-content.pdf", scanned: false, pages: 5)
'''

swift_path = ROOT / "_generate_pdfs.swift"
write(swift_path, swift_code)

try:
    subprocess.run(
        ["swift", str(swift_path), str(SRC)],
        check=True
    )
finally:
    if swift_path.exists():
        swift_path.unlink()

save_json("01-digital-text.json", {
    "pages": 6,
    "text_layer": "digital",
    "required_strings": ["PDF-PAGE-1-SONNE-2026"]
})

save_json("02-scanned.json", {
    "pages": 4,
    "text_layer": "image-only",
    "ocr_required": True
})

save_json("03-mixed-content.json", {
    "pages": 5,
    "text_layer": "digital",
    "note": "Поки цифровий PDF, не змішаний скан."
})


# --------------------------------------------------
# README AND VALIDATION
# --------------------------------------------------

write(ROOT / "README.md", """# Sonne Benchmark Corpus

Generated locally on macOS.

Directories:
- source: source documents
- ground-truth: expected data
- results: benchmark output

No cloud APIs or external Python packages used.

Known limitations:
- 03-mixed-content.pdf is currently digital-text only.
- DOCX image test uses text markers, not embedded images.
- PPTX contains text slides without illustrations.
- XLSX uses minimal OOXML without extensive visual formatting.

These limitations must be considered when interpreting benchmark results.
""")

expected = [
    "01-digital-text.pdf",
    "02-scanned.pdf",
    "03-mixed-content.pdf",
    "04-structured.docx",
    "05-images.docx",
    "06-multisheet.xlsx",
    "07-visual-slides.pptx",
    "08-semantic.html",
    "09-text-and-markdown.md",
    "10-data.csv",
    "11-book.epub",
    "12-nested-archive.zip"
]

missing = [
    name for name in expected
    if not (SRC / name).is_file() or (SRC / name).stat().st_size == 0
]

invalid = []
for name in [
    "04-structured.docx",
    "05-images.docx",
    "06-multisheet.xlsx",
    "07-visual-slides.pptx",
    "11-book.epub",
    "12-nested-archive.zip"
]:
    try:
        with zipfile.ZipFile(SRC / name) as archive:
            if archive.testzip():
                invalid.append(name)
    except Exception:
        invalid.append(name)

if missing or invalid:
    print("ПОМИЛКА ГЕНЕРАЦІЇ")
    print("Відсутні:", missing)
    print("Пошкоджені:", invalid)
    sys.exit(1)

print("\nГОТОВО")
print("Корпус:", ROOT)
print("Файлів:", len(expected))
print("Ground Truth:", len(list(GT.glob("*.json"))))
print("\nСтворені файли:")
for file in sorted(SRC.iterdir()):
    print(f"{file.name:30} {file.stat().st_size:>10} bytes")
print("\nУВАГА: переглянь README.md перед бенчмарками.")

