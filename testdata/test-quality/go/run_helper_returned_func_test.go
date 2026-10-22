// Label: t.Run(name, helper(..)) hands the subtest the closure the helper returns
// (cluster-api conversion tests). A helper defined in another file returns a body this parse
// cannot see (unknowable, quiet); a same-file helper is followed into its returned closure.
package fixtures

import "testing"

func TestRunHelperFromOtherPackage(t *testing.T) {
	t.Run("spoke", conversionutil.FuzzTestFunc(conversionutil.Input{Scheme: nil}))
}

func TestRunGenericHelperFromOtherPackage(t *testing.T) {
	t.Run("spoke", conversionutil.SpokeFuzz[*Hub, *Spoke](conversionutil.Input{Scheme: nil}))
}

func TestRunInlineEmptyClosure(t *testing.T) {
	t.Run("empty", func(t *testing.T) {})
}

func emptyBody() func(*testing.T) {
	return func(t *testing.T) {}
}

func TestRunSameFileEmptyClosure(t *testing.T) {
	t.Run("empty", emptyBody())
}

func assertingBody() func(*testing.T) {
	return func(t *testing.T) {
		t.Fatal("boom")
	}
}

func TestRunSameFileAssertingClosure(t *testing.T) {
	t.Run("asserts", assertingBody())
}
