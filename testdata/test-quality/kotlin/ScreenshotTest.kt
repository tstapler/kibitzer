// Label: negative. Golden-image tests: the screenshot comparison is the oracle, whether
// called directly, through a same-file helper, or through a project wrapper.
package fixtures

import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Test

class ScreenshotTest {
    @Test
    fun direct() {
        composeTestRule.onRoot().captureRoboImage("golden/direct.png")
    }

    @Test
    fun viaSameFileHelper() {
        captureAt(400, "golden/helper.png")
    }

    private fun captureAt(width: Int, name: String) {
        composeTestRule.onRoot().captureRoboImage(name)
    }

    @Test
    fun viaProjectWrapper() {
        composeTestRule.captureMultiTheme("Background") { Text("x") }
    }
}
