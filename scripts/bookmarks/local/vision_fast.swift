// Raw, full-page Apple Vision fast worker. No Preview UI, cloud or model downloads.
// PAGES.json NEW_OUTPUT_DIRECTORY; writes every completed page immediately.
import Foundation
import Vision
let args = CommandLine.arguments
guard args.count == 3 else { fatalError("vision_fast PAGES.json NEW_OUTPUT_DIRECTORY") }
let output = URL(fileURLWithPath: args[2], isDirectory: true)
guard !FileManager.default.fileExists(atPath: output.path) else { fatalError("output exists") }
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false)
let pages = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: args[1]))) as! [[String: Any]]
func box(_ b: CGRect) -> [Double] { [b.origin.x,b.origin.y,b.width,b.height] }
let pattern = try NSRegularExpression(pattern: "\\S+")
for page in pages {
    let began = Date()
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .fast
    request.usesLanguageCorrection = false
    let supported = try request.supportedRecognitionLanguages()
    request.recognitionLanguages = ["en-US","fr-FR","de-DE"].filter { supported.contains($0) }
    var row = page
    row["schema"] = "mpdf-vision-fast-raw/1"
    row["supported_languages"] = supported
    row["bbox_convention"] = "normalized_bottom_left_xywh"
    do {
        try VNImageRequestHandler(url: URL(fileURLWithPath: page["image_path"] as! String), options: [:]).perform([request])
        row["observations"] = (request.results ?? []).enumerated().compactMap { (i, obs) -> [String:Any]? in
            guard let candidate = obs.topCandidates(1).first else { return nil }
            let text = candidate.string
            let tokens: [[String:Any]] = pattern.matches(in: text, range: NSRange(text.startIndex..., in:text)).compactMap { match in
                guard let range = Range(match.range, in:text), let b = try? candidate.boundingBox(for: range) else { return nil }
                return ["text":String(text[range]),"bbox":box(b.boundingBox)]
            }
            return ["id":"obs-\(i)","text":text,"bbox":box(obs.boundingBox),"confidence":candidate.confidence,"tokens":tokens]
        }
        row["state"] = "locally_recognized"
    } catch {
        row["observations"] = [[String:Any]]()
        row["state"] = "failed"
        row["error"] = String(describing:error)
    }
    row["ocr_seconds"] = Date().timeIntervalSince(began)
    let data = try JSONSerialization.data(withJSONObject:row, options:[.prettyPrinted,.sortedKeys])
    try data.write(to:output.appendingPathComponent((page["id"] as! String)+".json"),options:.withoutOverwriting)
    print("\(page["id"]!): \(row["state"]!) \(row["ocr_seconds"]!)", terminator:"\n")
}
