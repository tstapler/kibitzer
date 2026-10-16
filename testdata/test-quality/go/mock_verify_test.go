// Label: true negative. Mock verification is the oracle.
package fixtures

import "testing"

func TestSave(t *testing.T) {
	repo := newMockRepo()
	svc := NewService(repo)
	svc.Create("x")
	repo.AssertExpectations(t)
}

func TestSaveCalled(t *testing.T) {
	repo := newMockRepo()
	NewService(repo).Create("x")
	repo.AssertCalled()
}
