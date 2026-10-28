// Label: known false negative: weak oracle. Zero findings from test-no-assertion
// and test-tautology; the gap is documented here, not silently widened.
package fixtures

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestWeak(t *testing.T) {
	got := Parse("")
	assert.NotNil(t, got)
}

func TestWeakLen(t *testing.T) {
	got := Parse("")
	assert.Greater(t, len(got), 0)
}
