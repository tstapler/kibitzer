// Label: six true positives in one file (no per-file cap applies before G1).
package fixtures

import "testing"

func TestV1(t *testing.T) { Parse("1") }
func TestV2(t *testing.T) { Parse("2") }
func TestV3(t *testing.T) { Parse("3") }
func TestV4(t *testing.T) { Parse("4") }
func TestV5(t *testing.T) { Parse("5") }
func TestV6(t *testing.T) { Parse("6") }
