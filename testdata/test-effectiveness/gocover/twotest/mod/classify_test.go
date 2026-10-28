package tec

import "testing"

func TestPos(t *testing.T) {
	if Classify(5) != "pos" {
		t.Fatal("want pos")
	}
}

func TestNeg(t *testing.T) {
	if Classify(-1) != "neg" {
		t.Fatal("want neg")
	}
}
