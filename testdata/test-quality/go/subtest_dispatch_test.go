// Label: true negatives. A same-file helper dispatches registered cases with
// t.Run(name, c.Run); the assertions live in the cases (ollama integration/reg_groups_test.go).
package fixtures

import "testing"

type regCase struct {
	Case string
	Run  func(t *testing.T)
}

var registry []regCase

func runGroup(t *testing.T, names ...string) {
	t.Helper()
	for _, c := range registry {
		c := c
		t.Run(c.Case, c.Run)
	}
	if len(names) == 0 {
		t.Skip("none selected")
	}
}

func TestDispatchViaFieldValue(t *testing.T) {
	runGroup(t, "a", "b")
}

type loudCase struct{ want int }

func (l loudCase) Check(t *testing.T) { t.Error("bad") }

func runLoud(t *testing.T, cases []loudCase) {
	for _, c := range cases {
		t.Run("x", c.Check)
	}
}

func TestDispatchToLoudMethod(t *testing.T) {
	runLoud(t, nil)
}
