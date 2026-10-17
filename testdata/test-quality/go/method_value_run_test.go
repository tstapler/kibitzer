// Label: true negatives. A method value passed to t.Run is a call target
// (kubernetes client-go dynamic/fake simple_test.go).
package fixtures

import "testing"

type patchCase struct {
	name string
	want int
}

func (tc *patchCase) runner(t *testing.T) {
	if tc.want != 1 {
		t.Error("bad")
	}
}

func TestMethodValueRun(t *testing.T) {
	cases := []patchCase{{name: "a", want: 1}}
	for _, tc := range cases {
		t.Run(tc.name, tc.runner)
	}
}

func subtest(t *testing.T) { t.Fatal("x") }

func TestFuncValueRun(t *testing.T) {
	t.Run("a", subtest)
}

func (tc *patchCase) quiet(t *testing.T) {}

// Method value with no assertion: still flagged.
func TestMethodValueNoAssertion(t *testing.T) {
	tc := patchCase{}
	t.Run("a", tc.quiet)
}
