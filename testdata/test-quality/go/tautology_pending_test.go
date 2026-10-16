// Label: test-tautology true positives (Story 1.1.3, not built yet). From
// test-no-assertion's view each test has an assertion call, so zero findings.
package fixtures

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestLiteralTrue(t *testing.T) {
	assert.True(t, true)
}

func TestSelfCompare(t *testing.T) {
	got := Parse("")
	assert.Equal(t, got, got)
}
