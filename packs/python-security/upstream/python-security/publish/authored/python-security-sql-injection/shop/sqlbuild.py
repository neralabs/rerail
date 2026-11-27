def build_where(term):
    return "SELECT * FROM shop_report WHERE title LIKE '%" + term + "%'"
