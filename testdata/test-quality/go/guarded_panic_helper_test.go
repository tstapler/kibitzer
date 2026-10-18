package fixtures

import "testing"

// Label: a panic under an `if` guard that checks behaviour is how a test fails without t.Fatal, in
// the test body, a goroutine, or a same-file helper. A bare panic asserts nothing, and neither
// does `if err != nil { panic(err) }` outside a goroutine: that is setup-error propagation.
func mustOK(err error) {
	if err != nil {
		panic(err)
	}
}

func expectLen(got, want int) {
	if got != want {
		panic("wrong length")
	}
}

func boom() { panic("unconditional") }

func TestGuardedPanicBody(_ *testing.T) {
	v := compute()
	if v != 0 {
		panic("must not change")
	}
}

func TestSetupErrPanicHelper(t *testing.T) { mustOK(compute()) }

func TestGuardedPanicCondHelper(t *testing.T) { expectLen(compute(), 1) }

func TestGuardedPanicGoroutineCond(t *testing.T) {
	done := make(chan struct{})
	go func() {
		defer close(done)
		if compute() > 3 {
			panic("too big")
		}
	}()
	<-done
}

func TestBarePanicHelper(t *testing.T) {
	compute()
	boom()
}

func TestBarePanicBody(t *testing.T) {
	compute()
	panic("unreachable")
}
