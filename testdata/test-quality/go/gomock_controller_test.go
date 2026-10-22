// Label: true negatives. A test that sets gomock `.EXPECT()` expectations on a controller made by
// NewController (directly or in a same-file suite helper whose teardown calls Finish) is verified
// when the controller finishes (tidb pkg/lightning/backend/backend_test.go TestClose). Reported:
// a suite with no expectations, and expectations with no controller in reach.
package fixtures

import (
	"testing"

	"go.uber.org/mock/gomock"
)

type backendSuite struct {
	controller  *gomock.Controller
	mockBackend *MockBackend
	backend     *Backend
}

func createBackendSuite(c gomock.TestReporter) *backendSuite {
	controller := gomock.NewController(c)
	return &backendSuite{controller: controller, mockBackend: NewMockBackend(controller)}
}

func (s *backendSuite) tearDownTest() {
	s.controller.Finish()
}

func TestClose(t *testing.T) {
	s := createBackendSuite(t)
	defer s.tearDownTest()

	s.mockBackend.EXPECT().Close().Return()

	s.backend.Close()
}

func TestSuiteWithoutExpectations(t *testing.T) {
	s := createBackendSuite(t)
	defer s.tearDownTest()

	s.backend.Close()
}

func TestExpectationsWithoutController(t *testing.T) {
	m := newFakeBackend()
	m.EXPECT().Close().Return()

	m.Close()
}
