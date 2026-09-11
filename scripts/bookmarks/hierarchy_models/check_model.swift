import Foundation
import FoundationModels
@main struct CheckModel {
    static func main() {
        let model = SystemLanguageModel.default
        let result: [String: Any] = ["available":model.isAvailable,
            "availability":String(describing:model.availability),
            "model":"SystemLanguageModel.default",
            "check":"availability_only_no_inference",
            "os":ProcessInfo.processInfo.operatingSystemVersionString]
        if let data = try? JSONSerialization.data(withJSONObject:result, options:.sortedKeys),
           let text = String(data:data, encoding:.utf8) { print(text) }
    }
}
