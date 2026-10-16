// Label: positive. Body only calls the code under test; no oracle.
package fixtures

import org.junit.Test

class EmptyTest {
    @Test
    fun parsesInput() {
        parse("")
    }

    @Test
    fun `backticked name without assertion`() {
        parse("x")
    }

    @Test
    fun emptyBody() {}
}
