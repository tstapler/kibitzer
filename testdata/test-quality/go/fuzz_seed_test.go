// Label: true negative. Fuzz seed test: the oracle is "does not crash".
package fixtures

import "testing"

func FuzzParse(f *testing.F) {
	f.Add("seed")
	f.Fuzz(func(t *testing.T, s string) {
		Parse(s)
	})
}
