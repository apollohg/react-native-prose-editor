const React = require('react');
const { Text } = require('react-native');

function IconMock(props) {
  const { name, children, ...rest } = props;
  return React.createElement(Text, rest, children ?? name ?? 'icon');
}

module.exports = {
  MaterialIcons: IconMock,
};
