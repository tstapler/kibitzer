package distincttypes

type BetaRepo struct{}

func (s *BetaRepo) Name() string { return "n" }

func describeB(svc *BetaRepo) string {
	if svc == nil {
		return "none"
	} else {
		return svc.Name()
	}
}
