@preconcurrency public import ExpoModulesCore
import ReactNativeProseEditor

public final class NativeCodeHighlightingModule: Module {
    private let provider = SyntectHighlightingProvider()

    public func definition() -> ModuleDefinition {
        Name("NativeCodeHighlighting")
        Function("initialize") { () throws -> Int in
            try NativeCodeHighlightingRegistry.register(provider: self.provider)
            return self.provider.version
        }
    }
}
