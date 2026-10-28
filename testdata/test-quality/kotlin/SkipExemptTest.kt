// Label: negative. Ignored tests and `@Test(expected = ...)` carry their own oracle.
package fixtures

import org.junit.Ignore
import org.junit.Test

class SkipExemptTest {
    @Test
    @Ignore("flaky")
    fun ignored() {
        parse("")
    }

    @Test(expected = IllegalStateException::class)
    fun expectedException() {
        parse("!")
    }
}
