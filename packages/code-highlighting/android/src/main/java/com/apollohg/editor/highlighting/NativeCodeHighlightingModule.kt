package com.apollohg.editor.highlighting

import com.apollohg.editor.CodeHighlightingRegistry
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class NativeCodeHighlightingModule : Module() {
    private val provider = SyntectHighlightingProvider()

    override fun definition() = ModuleDefinition {
        Name("NativeCodeHighlighting")
        Function("initialize") {
            CodeHighlightingRegistry.register(provider)
            provider.version
        }
    }
}
