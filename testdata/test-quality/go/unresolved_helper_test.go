// Label: strict mode flags TestExternalHelper (callee lives in another file);
// precision-first mode accepts it because the callee name is assertion-ish.
package fixtures

import "testing"

func TestExternalHelper(t *testing.T) {
	check.Valid(Parse(""))
}

// Passing the framework handle to an unresolved function counts as asserting.
func TestHandlePassed(t *testing.T) {
	otherpkg.Verify(t, Parse(""))
}
