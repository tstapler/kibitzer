package b

import "testing"

func TestParse(t *testing.T) {
	if !Parse("") {
		t.Fatal("want empty")
	}
}
