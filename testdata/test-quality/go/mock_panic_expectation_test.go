// Label: a mockery `.EXPECT()...Panic(...)` or testify `.On(...).Panic(...)` expectation fails
// the test when the forbidden call happens. A plain expectation without Panic, and a `Panic` on
// something that is not a mock expectation, are reported (precision-first accepts the former by its `EXPECT` name).
package fixtures

import "testing"

func TestNegativeExpectation(_ *testing.T) {
	cache := &mocks.ClusterCache{}
	cache.EXPECT().Invalidate(mock.Anything).Panic("should not invalidate").Maybe()
	handle(cache)
}

func TestOnPanic(_ *testing.T) {
	cache := &mocks.ClusterCache{}
	cache.On("Invalidate", mock.Anything).Panic("should not invalidate")
	handle(cache)
}

func TestExpectationWithoutPanic(_ *testing.T) {
	cache := &mocks.ClusterCache{}
	cache.EXPECT().Invalidate(mock.Anything).Return(nil).Maybe()
	handle(cache)
}

func TestPanicOnLogger(_ *testing.T) {
	logger.Panic("boom")
	handle(nil)
}
