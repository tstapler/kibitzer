package lib

func (s *Store) Inc() {
	s.mu.Lock()
	s.n++
	s.mu.Unlock()
}
