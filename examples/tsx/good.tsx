interface UserCardProps {
  firstName: string;
  lastName: string;
  email: string;
  phone: string;
  street: string;
  city: string;
}

function UserCard(props: UserCardProps) {
  const complete = props.firstName && props.lastName && props.email && props.phone && props.street;
  if (!complete) return <div>incomplete</div>;
  return (
    <div>
      {props.firstName} {props.lastName} {props.email} {props.phone} {props.street} {props.city}
    </div>
  );
}

function Report() {
  return <div>{sumRange(1, 41)}</div>;
}

function sumRange(from: number, to: number): number {
  let total = 0;
  for (let n = from; n <= to; n++) {
    total += n;
  }
  return total;
}
