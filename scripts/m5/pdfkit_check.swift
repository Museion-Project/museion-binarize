import Foundation
import PDFKit

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

guard CommandLine.arguments.count == 2 else {
    fail("usage: swift scripts/m5/pdfkit_check.swift <searchable.pdf>")
}

let url = URL(fileURLWithPath: CommandLine.arguments[1])
guard let document = PDFDocument(url: url) else {
    fail("PDFKit failed to open PDF")
}
guard document.pageCount == 4 else {
    fail("unexpected page count: \(document.pageCount)")
}

let text = (0 ..< document.pageCount)
    .compactMap { document.page(at: $0)?.string }
    .joined(separator: "\n")
for expected in ["Ἀρχὴ", "Πολιτείας", "2. Appendix iv"] {
    guard text.contains(expected) else {
        fail("missing searchable text: \(expected)")
    }
}

guard let root = document.outlineRoot else {
    fail("missing outline root")
}
var rows: [[String: Any]] = []
func walk(_ item: PDFOutline, level: Int) {
    for index in 0 ..< item.numberOfChildren {
        guard let child = item.child(at: index) else { continue }
        guard let destination = child.destination, let page = destination.page else {
            fail("outline destination missing: \(child.label ?? "<nil>")")
        }
        rows.append([
            "title": child.label ?? "",
            "level": level,
            "page_index": document.index(for: page),
            "x": destination.point.x,
            "y": destination.point.y,
        ])
        walk(child, level: level + 1)
    }
}
walk(root, level: 0)

let expected: [(String, Int, Int)] = [
    ("1. Ἀρχὴ", 0, 0),
    ("1.1 Πολιτείας", 1, 1),
    ("2. Appendix", 0, 3),
]
guard rows.count == expected.count else {
    fail("unexpected outline count: \(rows.count)")
}
for (index, item) in expected.enumerated() {
    guard rows[index]["title"] as? String == item.0,
          rows[index]["level"] as? Int == item.1,
          rows[index]["page_index"] as? Int == item.2
    else {
        fail("outline mismatch at \(index): \(rows[index])")
    }
}

let output: [String: Any] = [
    "engine": "Apple PDFKit",
    "page_count": document.pageCount,
    "unicode_text": true,
    "outline": rows,
]
let data = try JSONSerialization.data(
    withJSONObject: output,
    options: [.prettyPrinted, .sortedKeys]
)
print(String(data: data, encoding: .utf8)!)
