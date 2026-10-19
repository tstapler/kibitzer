// Label: true negatives. A function-value alias of a failing testing method counts when called
// (tailscale types/lazy/deferred_test.go). Reported: an alias that is never called.
package fixtures

import "testing"

func checkErr(tb testing.TB, got, want error, fatal bool) {
	tb.Helper()
	f := tb.Errorf
	if fatal {
		f = tb.Fatalf
	}
	if got != want {
		f("got %v want %v", got, want)
	}
}

func TestAliasShort(t *testing.T) {
	checkErr(t, nil, nil, false)
}

func TestAliasVar(t *testing.T) {
	var f = t.Errorf
	f("always")
}

func TestAliasUnused(t *testing.T) {
	f := t.Errorf
	_ = f
}
