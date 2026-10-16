// Label: negative. Every test uses a different Kotlin oracle form; none is flagged.
package fixtures

import app.cash.turbine.test
import io.mockk.coVerify
import io.mockk.verify
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.fail
import kotlinx.coroutines.test.runTest
import org.hamcrest.MatcherAssert.assertThat
import org.junit.Assert
import com.google.common.truth.Truth.assertThat

class OraclesTest {
    @Test
    fun kotlinTest() {
        assertEquals(3, add(1, 2))
    }

    @Test
    fun junitQualified() {
        Assert.assertTrue(parse("").isEmpty())
    }

    @Test
    fun truthChain() {
        assertThat(add(1, 2)).isEqualTo(3)
    }

    @Test
    fun hamcrest() {
        assertThat(add(1, 2), equalTo(3))
    }

    @Test
    fun mockkVerify() {
        run(service)
        verify { service.run() }
    }

    @Test
    fun mockkCoVerify() = runTest {
        run(service)
        coVerify(exactly = 1) { service.runAsync() }
    }

    @Test
    fun turbine() = runTest {
        flow.test {
            awaitItem()
            awaitComplete()
        }
    }

    @Test
    fun turbineExpectNoEvents() = runTest {
        flow.test { expectNoEvents() }
    }

    @Test
    fun failsWith() {
        assertFailsWith<IllegalStateException> { parse("!") }
    }

    @Test
    fun explicitFail() {
        try {
            parse("!")
            fail("expected a throw")
        } catch (e: IllegalStateException) {
        }
    }

    @Test
    fun runTestBody() = runTest {
        val got = loadAsync()
        assertEquals(1, got)
    }

    @Test
    fun insideRunBlocking() {
        runBlocking {
            assertEquals(1, loadAsync())
        }
    }

    @Test
    fun insideLambda() {
        listOf(1, 2).forEach { assertEquals(it, parse(it.toString())) }
    }

    @Test
    fun `backticked name with assertion`() {
        assertEquals(1, 1)
    }

    @Test
    fun expressionBody() = assertEquals(1, add(0, 1))

    @Test
    fun preconditionCheck() {
        check(parse("").isEmpty()) { "not empty" }
    }

    @Test
    fun kotestInfix() {
        parse("") shouldBe emptyList<String>()
    }
}
