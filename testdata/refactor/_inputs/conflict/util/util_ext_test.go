package util_test

import "testing"

// Util here would clash with the moved declaration.
func Util() string { return "t" }

func TestX(t *testing.T) { _ = Util() }
