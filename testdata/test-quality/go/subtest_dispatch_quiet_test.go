// Label: positive control for subtest dispatch. The dispatched same-file method asserts nothing,
// so the tests that only reach it are reported.
package fixtures

import "testing"

type quietCase struct{ name string }

func (q quietCase) Run(t *testing.T) {}

func runQuiet(t *testing.T, cases []quietCase) {
	for _, c := range cases {
		t.Run(c.name, c.Run)
	}
}

func TestDispatchToQuietMethod(t *testing.T) {
	runQuiet(t, nil)
}
