import re

from sqlalchemy import Column, ForeignKey, Integer
from sqlalchemy.orm import DeclarativeBase, declared_attr, relationship


class CatalogBase(DeclarativeBase):
    # table names derived from the class name: ProductFamily -> product_family
    @declared_attr.directive
    def __tablename__(cls):
        return re.sub(r"(?<!^)(?=[A-Z])", "_", cls.__name__).lower()


class ProductFamily(CatalogBase):
    id = Column(Integer, primary_key=True)


class CatalogItem(CatalogBase):
    id = Column(Integer, primary_key=True)
    product_family_id = Column(Integer, ForeignKey("product_family.id"))
    # many-to-one; the backref gives ProductFamily a `catalog_items` collection
    product_family = relationship("ProductFamily", backref="catalog_items")
