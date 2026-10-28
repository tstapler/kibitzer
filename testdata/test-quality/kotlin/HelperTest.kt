// Label: helpers. Same-file helper hops are followed two deep; an unresolved helper in
// another file is flagged in strict mode and accepted in precision-first mode when its name
// is assertion-ish.
package fixtures

import org.junit.Test

class HelperTest {
    @Test
    fun viaSameFileHelper() {
        checkParsed(parse(""))
    }

    private fun checkParsed(value: Any) {
        assertNotNull(value)
    }

    @Test
    fun viaTwoHopHelper() {
        outer(parse(""))
    }

    private fun outer(value: Any) = inner(value)

    private fun inner(value: Any) {
        assertNotNull(value)
    }

    @Test
    fun viaOtherFileHelper() {
        checkLoaded(parse(""))
    }

    @Test
    fun viaOtherFilePlainName() {
        load(parse(""))
    }
}
