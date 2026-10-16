// Label: helper resolution is bounded: TestShallow resolves, TestTooDeep does not.
package fixtures

import "testing"

func h1(t *testing.T) { h2(t) }
func h2(t *testing.T) { t.Fatal("x") }

func TestShallow(t *testing.T) {
	h1(t)
}

func d1() { d2() }
func d2() { d3() }
func d3() { d4() }
func d4() { t := new(testing.T); t.Fatal("deep") }

func TestTooDeep(t *testing.T) {
	d1()
}

// Recursive helpers must terminate.
func loop() { loop() }

func TestRecursive(t *testing.T) {
	loop()
}
