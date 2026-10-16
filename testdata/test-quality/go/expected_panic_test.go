// Label: true negative. Expected-failure oracle.
package fixtures

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestPanics(t *testing.T) {
	assert.Panics(t, func() { Parse("") })
}
