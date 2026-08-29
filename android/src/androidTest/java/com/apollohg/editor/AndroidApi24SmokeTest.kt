package com.apollohg.editor

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidApi24SmokeTest {
    @Test
    fun editorAndViewerSmoke() {
        ActivityScenario.launch(AndroidApi24SmokeActivity::class.java).use { scenario ->
            scenario.onActivity(AndroidApi24SmokeActivity::runApi24SmokeAssertions)
        }
    }
}
