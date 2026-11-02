package lib

import "testing"

func TestA(t *testing.T) {
	if b1() != 3 {
		t.Fatal("b1")
	}
}
