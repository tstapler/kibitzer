// Label: six vacuous tests for the per-file cap behaviour (UX-H7, H9).
package fixtures

import org.junit.Test

class SixVacuousTest {
    @Test fun a() { parse("a") }
    @Test fun b() { parse("b") }
    @Test fun c() { parse("c") }
    @Test fun d() { parse("d") }
    @Test fun e() { parse("e") }
    @Test fun f() { parse("f") }
}
