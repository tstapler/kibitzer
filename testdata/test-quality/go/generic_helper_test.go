// Label: true negatives. Helper called with explicit generic instantiation
// (kubernetes monotonic_test.go, unstructured_test.go).
package fixtures

import "testing"

func roundtripType[T any](t *testing.T) {
	var zero T
	if any(zero) != nil {
		t.Fatalf("unexpected %v", zero)
	}
}

func TestGenericHelper(t *testing.T) {
	roundtripType[int](t)
}

func TestGenericHelperPointer(t *testing.T) {
	roundtripType[*int](t)
}

func process[T any](x T) {}

// Generic helper without assertions: still flagged.
func TestGenericHelperNoAssertion(t *testing.T) {
	process[int](1)
}

func hiddenAssert[T any]() {
	var t *testing.T
	t.Fatal("deep")
}

// Resolved through the instantiation, no handle passed.
func TestGenericNoHandle(t *testing.T) {
	hiddenAssert[int]()
}
