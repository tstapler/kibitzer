// Label: known false negative: weak oracle. Zero findings from test-no-assertion;
// the gap is documented here, not silently widened.
package fixtures

import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class WeakOracleTest {
    @Test
    fun weak() {
        assertNotNull(parse(""))
    }

    @Test
    fun weakTautology() {
        assertTrue(true)
    }
}
