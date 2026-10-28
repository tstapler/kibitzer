package fixtures

import "testing"

// Label: a same-file function or method value handed to a call (as an argument or a struct-field
// value) is followed one level; its assertions count as the test's. A value that asserts nothing,
// or that is not defined in this file, keeps the test flagged.
type runArgs struct {
	t  *testing.T
	fn func(*testing.T)
}

func asserting(t *testing.T) {
	if got := compute(); got != 1 {
		t.Fatalf("got %d", got)
	}
}

func delegating(t *testing.T) { asserting(t) }

func silent(t *testing.T) { compute() }

type suite struct{}

func (s *suite) verify(t *testing.T) { t.Errorf("bad") }

func (s *suite) quiet(t *testing.T) { compute() }

func TestFuncValueArg(t *testing.T) { runner(asserting) }

func TestFuncValueStructField(t *testing.T) { execAPI(runArgs{t: t, fn: asserting}) }

func TestFuncValueDelegates(t *testing.T) { execAPI(runArgs{t: t, fn: delegating}) }

func TestMethodValueArg(t *testing.T) {
	s := &suite{}
	runner(s.verify)
}

func TestFuncValueSilent(t *testing.T) { execAPI(runArgs{t: t, fn: silent}) }

func TestFuncValueSilentArg(t *testing.T) { runner(silent) }

func TestMethodValueSilent(t *testing.T) {
	s := &suite{}
	runner(s.quiet)
}

func TestFuncValueNotInFile(t *testing.T) { execAPI(runArgs{t: t, fn: elsewhere}) }

func TestFuncValueNotInFileArg(t *testing.T) { runner(elsewhere) }
