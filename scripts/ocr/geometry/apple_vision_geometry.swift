import AppKit
import Foundation
import Vision

struct Line: Codable {
    let bbox: [Double]
    let confidence: Double
}

struct Result: Codable {
    let provider: String
    let providerVersion: String
    let width: Int
    let height: Int
    let lines: [Line]
}

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: apple_vision_geometry IMAGE\n".utf8))
    exit(2)
}

let imageURL = URL(fileURLWithPath: CommandLine.arguments[1])
guard let image = NSImage(contentsOf: imageURL),
      let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    FileHandle.standardError.write(Data("cannot decode image\n".utf8))
    exit(2)
}

let request = VNDetectTextRectanglesRequest()
request.reportCharacterBoxes = false

do {
    try VNImageRequestHandler(cgImage: cgImage, options: [:]).perform([request])
} catch {
    FileHandle.standardError.write(Data("Vision request failed: \(error.localizedDescription)\n".utf8))
    exit(1)
}

let width = Double(cgImage.width)
let height = Double(cgImage.height)
let lines = (request.results ?? []).map { observation -> Line in
    let box = observation.boundingBox
    return Line(
        bbox: [
            box.minX * width,
            (1.0 - box.maxY) * height,
            box.maxX * width,
            (1.0 - box.minY) * height,
        ],
        confidence: Double(observation.confidence)
    )
}
let version = ProcessInfo.processInfo.operatingSystemVersionString
let output = Result(
    provider: "apple-vision-text-rectangles",
    providerVersion: version,
    width: cgImage.width,
    height: cgImage.height,
    lines: lines
)
let encoder = JSONEncoder()
encoder.outputFormatting = [.sortedKeys]
do {
    FileHandle.standardOutput.write(try encoder.encode(output))
    FileHandle.standardOutput.write(Data("\n".utf8))
} catch {
    exit(1)
}
