# Label: validate() whose failure the except re-raises is an oracle; a swallowing handler is not.
def test_validate_reraises():
    for name, task in tasks.items():
        try:
            validate(instance=task, schema=schema)
        except Exception as e:
            print(f"Validation failed for {name}")
            raise e


def test_validate_swallowed():
    try:
        validate(instance=task, schema=schema)
    except Exception as e:
        print(e)


def test_reraise_without_validate():
    try:
        decide(event)
    except Exception as e:
        raise e
