package fires

type OrderService struct{}

func (s *OrderService) Name() string { return "n" }

func describeB(svc *OrderService) string {
	if svc == nil {
		return "none"
	} else {
		return svc.Name()
	}
}
