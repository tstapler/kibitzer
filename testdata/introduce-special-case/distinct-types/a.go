package distincttypes

type AlphaRepo struct{}

func (s *AlphaRepo) Name() string { return "n" }

func describeA(svc *AlphaRepo) string {
	if svc == nil {
		return "none"
	} else {
		return svc.Name()
	}
}
