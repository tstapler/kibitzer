package belowthreshold

type InvoiceService struct{}

func (s *InvoiceService) Name() string { return "n" }

func describeB(svc *InvoiceService) string {
	if svc == nil {
		return "none"
	} else {
		return svc.Name()
	}
}
