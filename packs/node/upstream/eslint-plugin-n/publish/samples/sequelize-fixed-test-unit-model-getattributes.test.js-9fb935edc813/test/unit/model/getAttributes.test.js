'use strict';

const chai = require('chai');
const expect = chai.expect;
const Support   = require('../support');
const current   = Support.sequelize;
const _ = require('lodash'),
  DataTypes = require('sequelize/lib/data-types');

  describe(Support.getTestDialectTeaser('Model'), () => {
    describe('getAttributes', () => {
      it('should return attributes with getAttributes()', () => {
        const Model = current.define('User', {}, {
          createdAt: 'createdAt',
          updatedAt: 'updatedAt',
          timestamps: true
        });  
        expect(Model.getAttributes()).to.haveOwnProperty('createdAt');
        expect(Model.getAttributes()).to.haveOwnProperty('updatedAt');
      }); 
    });
  });
  