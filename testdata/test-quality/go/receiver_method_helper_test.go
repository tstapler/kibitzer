// Label: true negatives. Assertions live in same-file methods reached through a receiver
// (kubernetes openapi controller_test.go) or a chain (endpointslice trigger_time_tracker_test.go).
package fixtures

import "testing"

type testEnv struct{ t *testing.T }

func (e *testEnv) expectPath(s string, path string) {
	if s != path {
		e.t.Errorf("missing %s", path)
	}
}

func TestReceiverMethod(t *testing.T) {
	env := &testEnv{t: t}
	env.expectPath("a", "a")
}

type tester struct{ t *testing.T }
type subject struct {
	got int
	t   *testing.T
}

func (r *tester) whenCompute() subject { return subject{1, r.t} }
func (s subject) expect(want int)      { s.doExpect(want) }
func (s subject) doExpect(want int) {
	if s.got != want {
		s.t.Errorf("got %d want %d", s.got, want)
	}
}

func TestChainedMethod(t *testing.T) {
	r := &tester{t: t}
	r.whenCompute().expect(1)
}

// A method that merely stores state is not an assertion: still flagged.
type recorder struct{ n int }

func (r *recorder) record() { r.n++ }

func TestRecorderOnly(t *testing.T) {
	r := &recorder{}
	r.record()
}

// A receiver that is the test handle never resolves to a same-file method of the same name.
type runner struct{}

func (runner) Run(t *testing.T) { t.Fatal("x") }

func TestHandleRunNotResolved(t *testing.T) {
	t.Run("sub", func(t *testing.T) { _ = 1 })
}

// A comment between chain links must not hide the resolved method (trigger_time_tracker_test.go).
func TestChainedMethodWithComment(t *testing.T) {
	r := &tester{t: t}
	r.whenCompute().
		// explain the expectation
		expect(1)
}
