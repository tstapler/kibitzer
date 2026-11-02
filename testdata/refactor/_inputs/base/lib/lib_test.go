package lib

import "testing"

func TestFoo(t *testing.T) {
	if Foo() == "" || helper() == "" {
		t.Fatal("empty")
	}
}
