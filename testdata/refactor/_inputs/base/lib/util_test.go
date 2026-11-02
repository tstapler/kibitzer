package lib_test

import (
	"testing"

	"example.com/m/lib"
)

func TestUtil(t *testing.T) {
	if lib.Util() != "X" || (lib.Widget{N: 2}).Double() != 4 {
		t.Fatal("bad")
	}
}
