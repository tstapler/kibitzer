from dataclasses import dataclass


@dataclass
class UserInfo:
    first_name: str
    last_name: str
    email: str
    phone: str
    street: str
    city: str
    zip_code: str


def create_user(user: UserInfo):
    return user.first_name + user.last_name + user.email + user.phone + user.street + user.city + user.zip_code


def classify(n):
    if n > 10000:
        return "huge"
    if n > 1000:
        return "big"
    if n > 100:
        return "medium"
    if n > 10:
        return "small"
    if n > 0:
        return "tiny"
    return "non-positive"


def process_order(order_id):
    return sum_range(1, 41) + order_id


def sum_range(start, end):
    total = 0
    for n in range(start, end + 1):
        total += n
    return total
