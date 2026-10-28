// Label: positive. A `capture*` call is an oracle only in a golden-image test file; here it
// just collects values and nothing is compared.
package fixtures

import org.junit.Test

class CaptureNonGoldenTest {
    @Test
    fun capturesLogs() {
        val logs = captureLogs { run() }
    }
}
