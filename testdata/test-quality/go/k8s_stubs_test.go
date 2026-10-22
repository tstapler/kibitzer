// Label: true positives that must stay flagged: empty TODO stubs (kubernetes cacher_test.go)
// and smoke-only tests (no-panic oracle).
package fixtures

import "testing"

func TestListInconsistentContinuation(t *testing.T) {
	// TODO(#109831): re-enable
}

func TestGuaranteedUpdate(t *testing.T) {
}

func TestTransformationFailure(t *testing.T) {
	// TODO(#109831)
}

func TestContextNoPanic(t *testing.T) {
	NewControllerDescriptors()
}

func TestSmokeObj(t *testing.T) {
	obj := newThing()
	obj.Start()
}
