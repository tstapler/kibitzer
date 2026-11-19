package distincttypes

type GammaRepo struct{}

func (s *GammaRepo) Name() string { return "n" }

func describeC(svc *GammaRepo) string {
	if svc == nil {
		return "none"
	} else {
		return svc.Name()
	}
}
