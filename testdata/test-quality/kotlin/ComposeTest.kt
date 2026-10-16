// Label: compose. Test-rule assertions are member calls on a finder chain; a click with no
// assertion is flagged.
package fixtures

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import org.junit.Rule
import org.junit.Test

class ComposeTest {
    @get:Rule
    val composeTestRule = createComposeRule()

    @Test
    fun showsTitle() {
        composeTestRule.setContent { Title("Hello") }
        composeTestRule.onNodeWithText("Hello").assertIsDisplayed()
    }

    @Test
    fun showsHelperTitle() {
        composeTestRule.setContent { Title("Hello") }
        assertTitleShown()
    }

    private fun assertTitleShown() {
        composeTestRule.onNodeWithText("Hello").assertIsDisplayed()
    }

    @Test
    fun clicksButtonWithoutAssertion() {
        composeTestRule.setContent { Title("Hello") }
        composeTestRule.onNodeWithText("Hello").performClick()
    }
}
