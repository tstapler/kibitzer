// Label: true positive. Body calls code but asserts nothing.
package fixtures

import "testing"

func TestA(t *testing.T) {
	Parse("")
}

func TestOk(t *testing.T) {
	if Parse("") == nil {
		t.Fatal("nil")
	}
}
