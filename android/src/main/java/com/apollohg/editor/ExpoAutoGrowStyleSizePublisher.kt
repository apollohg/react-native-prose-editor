package com.apollohg.editor

import expo.modules.kotlin.views.ExpoView

internal class ExpoAutoGrowStyleSizePublisher(private val view: ExpoView) {
    private data class Binding(
        val proxy: Any,
        val method: java.lang.reflect.Method,
        val stateWrapperGetter: java.lang.reflect.Method,
    )

    private val binding: Binding? by lazy(LazyThreadSafetyMode.NONE) {
        runCatching {
            val proxy = requireNotNull(
                view.javaClass.methods
                    .first { it.name == "getShadowNodeProxy" && it.parameterCount == 0 }
                    .invoke(view)
            )
            val method = proxy.javaClass.methods
                .first { it.name == "setStyleSize" && it.parameterCount == 2 }
            val stateWrapperGetter = view.javaClass.methods
                .first { it.name == "getStateWrapper" && it.parameterCount == 0 }
            Binding(proxy, method, stateWrapperGetter)
        }.getOrNull()
    }

    fun publish(heightDp: Double?): Boolean {
        val resolvedBinding = binding ?: return false
        return runCatching {
            if (resolvedBinding.stateWrapperGetter.invoke(view) == null) return false
            resolvedBinding.method.invoke(resolvedBinding.proxy, null, heightDp)
        }.isSuccess
    }
}
