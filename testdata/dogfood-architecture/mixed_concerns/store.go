package mixed_concerns

type Store struct{ rows []string }

type Record struct{ id string }

func (s *Store) Put(r *Record) { s.rows = append(s.rows, r.ID()) }

func (r *Record) ID() string { return r.id }

func Save(s *Store, r *Record) { s.Put(r) }
