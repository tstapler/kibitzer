// Label: the testing handle's assertion METHOD passed as a function value into a same-file
// helper is an assertion call site when the helper invokes that parameter (or cannot be resolved).
package fixtures

import "testing"

type errorf func(string, ...any)

func apply(ef errorf, got, want string) { applyWithPath(ef, got, want) }

func applyWithPath(ef errorf, got, want string) {
	if got != want {
		ef("got %q want %q", got, want)
	}
}

func check(fail func(string, ...any), got int) {
	if got != 1 {
		fail("bad %d", got)
	}
}

func each(items []string, report func(...any)) {
	for _, it := range items {
		func() { report(it) }()
	}
}

func fatalOn(stop func(), bad bool) {
	if bad {
		stop()
	}
}

func TestViaTwoHops(t *testing.T)      { apply(t.Errorf, "a", "a") }
func TestDirectInvoke(t *testing.T)    { check(t.Fatalf, 1) }
func TestInvokedInClosure(t *testing.T) { each([]string{"x"}, t.Error) }
func TestFailNowCallback(t *testing.T) { fatalOn(t.FailNow, false) }

func TestUnresolvedReceiver(t *testing.T) {
	otherpkg.Compare(t.Errorf, "a", "a")
}

func TestSubtestHandle(t *testing.T) {
	t.Run("x", func(t *testing.T) { check(t.Fatalf, 1) })
}

// Negative: the helper takes the callback but never calls it.
func ignoresCallback(ef errorf, got, want string) { _ = got == want }

func passesToIgnorer(ef errorf, got, want string) { ignoresCallback(ef, got, want) }

func TestHelperNeverCalls(t *testing.T) { ignoresCallback(t.Errorf, "a", "b") }

func TestIgnoredTwoHops(t *testing.T) { passesToIgnorer(t.Errorf, "a", "b") }

// Negative: Logf is not an oracle.
func TestPassesLogf(t *testing.T) { each([]string{"x"}, t.Log) }

func TestPassesLogfHelper(t *testing.T) { apply(t.Logf, "a", "b") }

// Cycle safety: mutually recursive helpers that never call the parameter terminate.
func ping(ef errorf, n int) { pong(ef, n-1) }

func pong(ef errorf, n int) { ping(ef, n-1) }

func TestCycleNeverCalls(t *testing.T) { ping(t.Errorf, 3) }

// A panic guarded by an error check inside a goroutine is an oracle (t.Fatal is unsafe there),
// whether the goroutine is in the test or in a same-file helper. In setup code it is not.
func mustRun(f func() error) {
	if err := f(); err != nil {
		panic(err)
	}
}

func spawnChecked(f func() error) {
	go func() {
		if err := f(); err != nil {
			panic(err)
		}
	}()
}

func TestGuardedPanicGoroutine(t *testing.T) {
	done := make(chan struct{})
	go func() {
		defer close(done)
		if err := run(); err != nil {
			panic(err)
		}
	}()
	<-done
}

func TestGuardedPanicGoroutineHelper(t *testing.T) { spawnChecked(run) }

// Negative: setup-style panic on error outside any goroutine.
func TestGuardedPanicSetupOnly(t *testing.T) { mustRun(run) }

// Negative: an unconditional or non-error-guarded panic asserts nothing.
func TestUnguardedPanic(t *testing.T) {
	run()
	panic("unreachable")
}

func TestPanicOnFlag(t *testing.T) {
	run()
	if verbose {
		panic("verbose")
	}
}
