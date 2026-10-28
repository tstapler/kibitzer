// Label: true negatives. A test entry point that calls testify's suite.Run delegates to the
// suite's methods (temporal service/frontend/operator_handler_test.go). Reported: a Run on a
// receiver that is not testify's suite package.
package fixtures

import (
	"testing"

	"github.com/stretchr/testify/suite"
	tsuite "github.com/stretchr/testify/suite"
)

type exampleSuite struct{ suite.Suite }

func (s *exampleSuite) TestOne() { s.Equal(1, 1) }

func TestExampleSuite(t *testing.T) {
	s := new(exampleSuite)
	suite.Run(t, s)
}

func TestExampleSuiteAliased(t *testing.T) {
	tsuite.Run(t, new(exampleSuite))
}

type runner struct{}

func (runner) Run(t *testing.T, v any) {}

func TestRunOnNonSuite(t *testing.T) {
	r := runner{}
	r.Run(t, 1)
}
